use crate::error::CliError;
use mallard_application::storage_setup::SecretString;
use mallard_domain::HostId;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const BROKER_URL: &str = "https://oauth.pipelines.cloudflare.com/oauth/login";
const BROKER_CALLBACK_ADDRESS: &str = "localhost:8976";
const BROKER_WELCOME_URL: &str =
    "https://welcome.developers.workers.dev/wrangler-oauth-consent-granted";
const MAX_WRANGLER_JSON_BYTES: u64 = 256 * 1024;
const MAX_CALLBACK_REQUEST_BYTES: usize = 16 * 1024;
const MAX_CALLBACK_BODY_BYTES: usize = 512;
const MAX_CALLBACK_CREDENTIAL_BYTES: usize = 4 * 1024;
const MAX_CALLBACK_REQUESTS: usize = 16;
const FLOW_TOKEN_BYTES: usize = 32;
const FLOW_TOKEN_HEX_BYTES: usize = FLOW_TOKEN_BYTES * 2;
const R2_BUCKET_MAX_BYTES: usize = 63;
const R2_BUCKET_PREFIX: &str = "mallard-";
const R2_BUCKET_SUFFIX_BYTES: usize = 12;
const R2_BUCKET_SUFFIX_HEX_BYTES: usize = R2_BUCKET_SUFFIX_BYTES * 2;
const R2_BUCKET_SLUG_MAX_BYTES: usize =
    R2_BUCKET_MAX_BYTES - R2_BUCKET_PREFIX.len() - 1 - R2_BUCKET_SUFFIX_HEX_BYTES;
const R2_BUCKET_FALLBACK_SLUG: &str = "storage";
const START_PATH_PREFIX: &str = "/mallard/r2/start/";
const COMPLETION_PATH: &str = "/mallard/r2/complete";
const SAFE_CALLBACK_PATH: &str = "/mallard/r2/authorizing";
const FLOW_SESSION_KEY: &str = "mallard:r2:browser-flow";
const WHOAMI_TIMEOUT: Duration = Duration::from_secs(30);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
const BUCKET_TIMEOUT: Duration = Duration::from_secs(120);
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);
const CALLBACK_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const PENDING_CREDENTIAL_TIMEOUT: Duration = Duration::from_secs(30);

const REMOVED_WRANGLER_ENVIRONMENT_VARIABLES: &[&str] = &[
    "CLOUDFLARE_API_TOKEN",
    "CLOUDFLARE_API_KEY",
    "CLOUDFLARE_EMAIL",
    "CF_API_TOKEN",
    "CF_API_KEY",
    "CF_EMAIL",
    "WRANGLER_API_TOKEN",
    "WRANGLER_CF_AUTHORIZATION_TOKEN",
    "CLOUDFLARE_ACCESS_CLIENT_ID",
    "CLOUDFLARE_ACCESS_CLIENT_SECRET",
    "CLOUDFLARE_ACCOUNT_ID",
    "CF_ACCOUNT_ID",
    "CLOUDFLARE_API_BASE_URL",
    "CF_API_BASE_URL",
    "WRANGLER_API_ENVIRONMENT",
    "CLOUDFLARE_COMPLIANCE_REGION",
    "WRANGLER_AUTH_DOMAIN",
    "WRANGLER_AUTH_URL",
    "WRANGLER_TOKEN_URL",
    "WRANGLER_REVOKE_URL",
    "WRANGLER_CLIENT_ID",
    "WRANGLER_OUTPUT_FILE_PATH",
    "WRANGLER_OUTPUT_FILE_DIRECTORY",
    "WRANGLER_LOG_PATH",
    "WRANGLER_LOG",
];

pub(crate) struct R2Destination {
    pub account_id: String,
    pub bucket: String,
    pub api_url: String,
}

pub(crate) struct BrowserCredentials {
    access_key_id: Zeroizing<String>,
    secret_access_key: Zeroizing<String>,
}

impl BrowserCredentials {
    pub fn access_key_id(&self) -> String {
        self.access_key_id.to_string()
    }

    pub fn secret_access_key(&self) -> SecretString {
        SecretString::new(self.secret_access_key.to_string())
    }
}

pub(crate) struct WranglerClient {
    program: OsString,
    workdir: TempDir,
}

impl WranglerClient {
    pub fn new() -> Result<Self, CliError> {
        Self::with_program(OsString::from("wrangler"))
    }

    fn with_program(program: OsString) -> Result<Self, CliError> {
        let workdir = tempfile::Builder::new()
            .prefix("mallard-wrangler-")
            .tempdir()
            .map_err(|_| CliError::persistence("Mallard could not prepare Cloudflare setup."))?;
        Ok(Self { program, workdir })
    }

    pub fn authenticated_account_id(&self) -> Result<String, CliError> {
        match self.whoami()? {
            AuthState::Authenticated(account_id) => Ok(account_id),
            AuthState::LoggedOut => {
                let logged_in = self
                    .run_status(&["login"], None, LOGIN_TIMEOUT)
                    .map_err(map_wrangler_auth_process_error)?;
                if !logged_in {
                    return Err(CliError::remote(
                        "wrangler_login_failed",
                        "Cloudflare login was not completed.",
                    ));
                }
                match self.whoami()? {
                    AuthState::Authenticated(account_id) => Ok(account_id),
                    AuthState::LoggedOut => Err(CliError::remote(
                        "wrangler_login_failed",
                        "Cloudflare login was not completed.",
                    )),
                }
            }
        }
    }

    pub fn ensure_bucket(&self, account_id: &str, bucket: &str) -> Result<(), CliError> {
        let created = self
            .run_status(
                &["r2", "bucket", "create", bucket, "--update-config=false"],
                Some(account_id),
                BUCKET_TIMEOUT,
            )
            .map_err(map_wrangler_bucket_process_error)?;
        if created {
            return Ok(());
        }

        let exists = self
            .run_status(
                &["r2", "bucket", "info", bucket, "--json"],
                Some(account_id),
                BUCKET_TIMEOUT,
            )
            .map_err(map_wrangler_bucket_process_error)?;
        if exists {
            Ok(())
        } else {
            Err(CliError::remote(
                "wrangler_r2_bucket_failed",
                "Wrangler could not create or access the Cloudflare R2 bucket.",
            ))
        }
    }

    fn whoami(&self) -> Result<AuthState, CliError> {
        let (status, output) = self
            .run_json(&["whoami", "--json"], WHOAMI_TIMEOUT)
            .map_err(map_wrangler_auth_process_error)?;
        parse_whoami(status.success(), &output)
    }

    fn run_json(
        &self,
        args: &[&str],
        timeout: Duration,
    ) -> Result<(ExitStatus, Zeroizing<Vec<u8>>), ProcessFailure> {
        let mut command = self.command(args, None);
        command.stdout(Stdio::piped());
        let mut child = command.spawn().map_err(map_spawn_failure)?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProcessFailure::Unavailable);
        };
        let reader = match thread::Builder::new()
            .name("mallard-wrangler-output".to_string())
            .spawn(move || read_bounded_stream(stdout))
        {
            Ok(reader) => reader,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessFailure::Unavailable);
            }
        };
        let status = wait_with_timeout(&mut child, timeout);
        let output = reader.join().map_err(|_| ProcessFailure::Unavailable)?;
        Ok((status?, output?))
    }

    fn run_status(
        &self,
        args: &[&str],
        account_id: Option<&str>,
        timeout: Duration,
    ) -> Result<bool, ProcessFailure> {
        let mut command = self.command(args, account_id);
        command.stdout(Stdio::null());
        let mut child = command.spawn().map_err(map_spawn_failure)?;
        wait_with_timeout(&mut child, timeout).map(|status| status.success())
    }

    fn command(&self, args: &[&str], account_id: Option<&str>) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .current_dir(self.workdir.path())
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .env("NO_COLOR", "1")
            .env("WRANGLER_SEND_METRICS", "false")
            .env("WRANGLER_SEND_ERROR_REPORTS", "false")
            .env("WRANGLER_NO_SKILLS_UPDATE_PROMPTS", "true")
            .env("WRANGLER_WRITE_LOGS", "false")
            .env_remove("CI")
            .env_remove("WRANGLER_LOG_SANITIZE");
        for variable in REMOVED_WRANGLER_ENVIRONMENT_VARIABLES {
            command.env_remove(variable);
        }
        command.env("WRANGLER_LOG_SANITIZE", "true");
        if let Some(account_id) = account_id {
            command.env("CLOUDFLARE_ACCOUNT_ID", account_id);
        }
        command
    }
}

#[derive(Deserialize)]
struct WhoamiDocument {
    #[serde(rename = "loggedIn")]
    logged_in: bool,
    #[serde(default)]
    accounts: Vec<WhoamiAccount>,
}

#[derive(Deserialize)]
struct WhoamiAccount {
    id: String,
}

enum AuthState {
    LoggedOut,
    Authenticated(String),
}

enum ProcessFailure {
    Missing,
    TimedOut,
    OutputTooLarge,
    Unavailable,
}

fn parse_whoami(success: bool, bytes: &[u8]) -> Result<AuthState, CliError> {
    let document = serde_json::from_slice::<WhoamiDocument>(bytes).map_err(|_| {
        CliError::remote(
            "wrangler_auth_unavailable",
            "Wrangler authentication could not be verified.",
        )
    })?;
    if !document.logged_in {
        return Ok(AuthState::LoggedOut);
    }
    if !success || document.accounts.is_empty() || document.accounts.len() > 64 {
        return Err(CliError::remote(
            "wrangler_auth_unavailable",
            "Wrangler authentication could not be verified.",
        ));
    }

    let mut account_ids = BTreeSet::new();
    for mut account in document.accounts {
        if account.id.len() != 32 || !account.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            account.id.zeroize();
            return Err(CliError::remote(
                "wrangler_auth_unavailable",
                "Wrangler authentication could not be verified.",
            ));
        }
        account_ids.insert(account.id.to_ascii_lowercase());
        account.id.zeroize();
    }
    if account_ids.len() != 1 {
        return Err(CliError::invalid_code(
            "wrangler_account_ambiguous",
            "Wrangler is authenticated to multiple Cloudflare accounts, so Mallard cannot choose one automatically.",
        ));
    }
    Ok(AuthState::Authenticated(
        account_ids
            .pop_first()
            .expect("one authenticated account remains"),
    ))
}

fn map_spawn_failure(error: std::io::Error) -> ProcessFailure {
    if error.kind() == std::io::ErrorKind::NotFound {
        ProcessFailure::Missing
    } else {
        ProcessFailure::Unavailable
    }
}

fn map_wrangler_auth_process_error(error: ProcessFailure) -> CliError {
    match error {
        ProcessFailure::Missing => CliError::invalid_code(
            "wrangler_not_found",
            "Wrangler is required on PATH. Install Wrangler and run this command again.",
        ),
        ProcessFailure::TimedOut => CliError::remote(
            "wrangler_auth_timeout",
            "Cloudflare authentication timed out.",
        ),
        ProcessFailure::OutputTooLarge | ProcessFailure::Unavailable => CliError::remote(
            "wrangler_auth_unavailable",
            "Wrangler authentication could not be verified.",
        ),
    }
}

fn map_wrangler_bucket_process_error(error: ProcessFailure) -> CliError {
    match error {
        ProcessFailure::Missing => CliError::invalid_code(
            "wrangler_not_found",
            "Wrangler is required on PATH. Install Wrangler and run this command again.",
        ),
        ProcessFailure::TimedOut => CliError::remote(
            "wrangler_r2_bucket_timeout",
            "Wrangler timed out while preparing Cloudflare R2.",
        ),
        ProcessFailure::OutputTooLarge | ProcessFailure::Unavailable => CliError::remote(
            "wrangler_r2_bucket_failed",
            "Wrangler could not create or access the Cloudflare R2 bucket.",
        ),
    }
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<ExitStatus, ProcessFailure> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessFailure::TimedOut);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ProcessFailure::Unavailable);
            }
        }
    }
}

fn read_bounded_stream(reader: impl Read) -> Result<Zeroizing<Vec<u8>>, ProcessFailure> {
    let mut bytes = Zeroizing::new(Vec::with_capacity(8 * 1024));
    reader
        .take(MAX_WRANGLER_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ProcessFailure::Unavailable)?;
    if bytes.len() as u64 > MAX_WRANGLER_JSON_BYTES {
        return Err(ProcessFailure::OutputTooLarge);
    }
    Ok(bytes)
}

pub(crate) fn derive_destination(account_id: &str, storage_name: &str) -> R2Destination {
    let mut hasher = Sha256::new();
    hasher.update(b"mallard-r2-bucket-v2\0");
    hasher.update(account_id.as_bytes());
    hasher.update([0]);
    hasher.update(storage_name.as_bytes());
    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(R2_BUCKET_SUFFIX_HEX_BYTES);
    for byte in digest.iter().take(R2_BUCKET_SUFFIX_BYTES) {
        use std::fmt::Write as _;
        write!(&mut suffix, "{byte:02x}").expect("writing to a String cannot fail");
    }
    let readable_slug = readable_bucket_slug(storage_name);
    let bucket = format!("{R2_BUCKET_PREFIX}{readable_slug}-{suffix}");
    R2Destination {
        account_id: account_id.to_string(),
        api_url: format!("https://{account_id}.r2.cloudflarestorage.com/{bucket}"),
        bucket,
    }
}

fn readable_bucket_slug(storage_name: &str) -> String {
    let mut slug = String::with_capacity(R2_BUCKET_SLUG_MAX_BYTES);
    let mut separator_pending = false;

    for byte in storage_name.bytes() {
        if !byte.is_ascii_alphanumeric() {
            separator_pending = !slug.is_empty();
            continue;
        }

        if separator_pending {
            if slug.len() + 1 >= R2_BUCKET_SLUG_MAX_BYTES {
                break;
            }
            slug.push('-');
        }
        if slug.len() >= R2_BUCKET_SLUG_MAX_BYTES {
            break;
        }
        slug.push(byte.to_ascii_lowercase() as char);
        separator_pending = false;
    }

    if slug.is_empty() {
        R2_BUCKET_FALLBACK_SLUG.to_string()
    } else {
        slug
    }
}

pub(crate) fn authorize_bucket(
    account_id: &str,
    bucket: &str,
) -> Result<BrowserCredentials, CliError> {
    let listener = bind_callback_listener()?;
    let url = broker_url(account_id, bucket);
    authorize_with_listener(
        listener,
        &url,
        |url| open::that_detached(url.as_str()).map_err(|_| ()),
        CALLBACK_TIMEOUT,
    )
}

fn bind_callback_listener() -> Result<TcpListener, CliError> {
    let address = BROKER_CALLBACK_ADDRESS
        .to_socket_addrs()
        .map_err(|_| callback_listener_error())?
        .find(|address| address.ip().is_loopback())
        .ok_or_else(callback_listener_error)?;
    let listener = bind_reusable_listener(address).map_err(|_| callback_listener_error())?;
    let bound = listener
        .local_addr()
        .map_err(|_| callback_listener_error())?;
    if !bound.ip().is_loopback() {
        return Err(callback_listener_error());
    }
    Ok(listener)
}

fn bind_reusable_listener(address: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = Socket::new(
        Domain::for_address(address),
        Type::STREAM,
        Some(Protocol::TCP),
    )?;
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    #[cfg(windows)]
    socket.set_exclusive_address_use(true)?;
    socket.bind(&address.into())?;
    socket.listen(128)?;
    Ok(socket.into())
}

fn callback_listener_error() -> CliError {
    CliError::remote(
        "r2_authorization_callback_unavailable",
        "Mallard could not start the local Cloudflare authorization callback.",
    )
}

fn broker_url(account_id: &str, bucket: &str) -> Url {
    let mut url = Url::parse(BROKER_URL).expect("the Cloudflare broker URL is static and valid");
    url.query_pairs_mut()
        .append_pair("accountId", account_id)
        .append_pair("bucketName", bucket)
        .append_pair("pipelineName", bucket);
    url
}

fn authorize_with_listener<F>(
    listener: TcpListener,
    broker_url: &Url,
    opener: F,
    timeout: Duration,
) -> Result<BrowserCredentials, CliError>
where
    F: FnOnce(&Url) -> Result<(), ()>,
{
    listener.set_nonblocking(true).map_err(|_| {
        CliError::remote(
            "r2_authorization_callback_unavailable",
            "Mallard could not start the local Cloudflare authorization callback.",
        )
    })?;
    let port = listener
        .local_addr()
        .map_err(|_| callback_listener_error())?
        .port();
    let mut flow = BrowserAuthorizationFlow::new(format!("localhost:{port}"), broker_url.clone())?;
    let start_url = flow.start_url();
    opener(&start_url).map_err(|_| {
        CliError::remote(
            "r2_authorization_browser_failed",
            "Mallard could not open the Cloudflare authorization page.",
        )
    })?;
    receive_credentials(&listener, &mut flow, timeout)
}

struct BrowserAuthorizationFlow {
    expected_host: String,
    broker_url: Url,
    flow_id: Zeroizing<String>,
    start_path: Zeroizing<String>,
    start_available: bool,
    pending: Option<PendingCredentials>,
}

struct PendingCredentials {
    handle: Zeroizing<String>,
    credentials: BrowserCredentials,
    expires_at: Instant,
}

impl BrowserAuthorizationFlow {
    fn new(expected_host: String, broker_url: Url) -> Result<Self, CliError> {
        let flow_id = random_256_bit_token()?;
        let start_path = Zeroizing::new(format!("{START_PATH_PREFIX}{}", flow_id.as_str()));
        Ok(Self {
            expected_host,
            broker_url,
            flow_id,
            start_path,
            start_available: true,
            pending: None,
        })
    }

    fn start_url(&self) -> Url {
        Url::parse(&format!(
            "http://{}{}",
            self.expected_host,
            self.start_path.as_str()
        ))
        .expect("the bound loopback origin and generated start path are valid")
    }

    fn start_page(&self) -> Zeroizing<String> {
        start_page(&self.flow_id, &self.broker_url)
    }

    fn stage_credentials(
        &mut self,
        credentials: BrowserCredentials,
    ) -> Result<Zeroizing<String>, CliError> {
        let handle = random_256_bit_token()?;
        let page = completion_page(&handle);
        self.pending = Some(PendingCredentials {
            handle,
            credentials,
            expires_at: Instant::now() + PENDING_CREDENTIAL_TIMEOUT,
        });
        Ok(page)
    }

    fn complete(
        &mut self,
        presented_handle: &str,
        presented_flow_id: &str,
        now: Instant,
    ) -> Option<BrowserCredentials> {
        self.clear_expired_pending(now);
        let pending = self.pending.as_ref()?;
        let handle_matches = constant_time_token_eq(&pending.handle, presented_handle);
        let flow_matches = constant_time_token_eq(&self.flow_id, presented_flow_id);
        if !(handle_matches & flow_matches) {
            return None;
        }
        self.pending.take().map(|pending| pending.credentials)
    }

    fn clear_expired_pending(&mut self, now: Instant) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| now >= pending.expires_at)
        {
            self.pending = None;
        }
    }
}

fn random_256_bit_token() -> Result<Zeroizing<String>, CliError> {
    let first = Zeroizing::new(String::from(
        HostId::generate().map_err(|_| callback_failed_error())?,
    ));
    let second = Zeroizing::new(String::from(
        HostId::generate().map_err(|_| callback_failed_error())?,
    ));
    let mut token = Zeroizing::new(String::with_capacity(FLOW_TOKEN_HEX_BYTES));
    token.push_str(&first);
    token.push_str(&second);
    debug_assert_eq!(token.len(), FLOW_TOKEN_HEX_BYTES);
    Ok(token)
}

#[inline(never)]
fn constant_time_token_eq(expected: &str, presented: &str) -> bool {
    if expected.len() != FLOW_TOKEN_HEX_BYTES || presented.len() != FLOW_TOKEN_HEX_BYTES {
        return false;
    }
    let mut difference = 0u8;
    for (expected, presented) in expected.bytes().zip(presented.bytes()) {
        difference |= expected ^ presented;
    }
    std::hint::black_box(difference) == 0
}

fn receive_credentials(
    listener: &TcpListener,
    flow: &mut BrowserAuthorizationFlow,
    timeout: Duration,
) -> Result<BrowserCredentials, CliError> {
    let deadline = Instant::now() + timeout;
    let mut requests = 0usize;
    loop {
        flow.clear_expired_pending(Instant::now());
        if Instant::now() >= deadline {
            return Err(CliError::remote(
                "r2_authorization_timeout",
                "Cloudflare authorization timed out.",
            ));
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                requests += 1;
                if !peer.ip().is_loopback() {
                    let _ = write_http_response(&mut stream, 403);
                } else {
                    let result = read_callback_request(&mut stream, deadline).and_then(|request| {
                        parse_callback_request(&request, &flow.expected_host, &flow.start_path)
                    });
                    match result {
                        Ok(CallbackAction::Start) if flow.start_available => {
                            let page = flow.start_page();
                            if write_html_response(&mut stream, &page, &flow.flow_id, false).is_ok()
                            {
                                flow.start_available = false;
                            }
                        }
                        Ok(CallbackAction::Start) => {
                            let _ = write_http_response(&mut stream, 404);
                        }
                        Ok(CallbackAction::Credentials(credentials)) if !flow.start_available => {
                            let page = flow.stage_credentials(credentials)?;
                            let handle = flow
                                .pending
                                .as_ref()
                                .expect("staging credentials installs one pending handle")
                                .handle
                                .as_str();
                            let _ = write_html_response(&mut stream, &page, handle, true);
                        }
                        Ok(CallbackAction::Credentials(_)) => {
                            let _ = write_http_response(&mut stream, 403);
                        }
                        Ok(CallbackAction::Complete { handle, flow_id }) => {
                            if let Some(credentials) =
                                flow.complete(&handle, &flow_id, Instant::now())
                            {
                                write_http_response(&mut stream, 204)
                                    .map_err(|_| callback_failed_error())?;
                                return Ok(credentials);
                            }
                            let _ = write_http_response(&mut stream, 403);
                        }
                        Err(error) => {
                            let _ = write_http_response(&mut stream, error.status_code());
                        }
                    }
                }
                if requests >= MAX_CALLBACK_REQUESTS {
                    return Err(CliError::remote(
                        "r2_authorization_callback_failed",
                        "Mallard did not receive a valid Cloudflare authorization callback.",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                return Err(CliError::remote(
                    "r2_authorization_callback_failed",
                    "Mallard could not receive the Cloudflare authorization callback.",
                ));
            }
        }
    }
}

enum CallbackAction {
    Start,
    Credentials(BrowserCredentials),
    Complete {
        handle: Zeroizing<String>,
        flow_id: Zeroizing<String>,
    },
}

struct CallbackHttpRequest {
    bytes: Zeroizing<Vec<u8>>,
    header_end: usize,
    body_length: usize,
    content_length_present: bool,
}

impl CallbackHttpRequest {
    fn head(&self) -> &[u8] {
        &self.bytes[..self.header_end]
    }

    fn body(&self) -> &[u8] {
        &self.bytes[self.header_end..]
    }
}

#[derive(Clone, Copy)]
enum CallbackRequestError {
    BadRequest,
    MethodNotAllowed,
    NotFound,
}

impl CallbackRequestError {
    fn status_code(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::MethodNotAllowed => 405,
            Self::NotFound => 404,
        }
    }
}

fn read_callback_request(
    stream: &mut TcpStream,
    callback_deadline: Instant,
) -> Result<CallbackHttpRequest, CallbackRequestError> {
    let request_deadline = callback_deadline.min(Instant::now() + CALLBACK_REQUEST_TIMEOUT);
    stream
        .set_write_timeout(Some(CALLBACK_REQUEST_TIMEOUT))
        .map_err(|_| CallbackRequestError::BadRequest)?;
    let mut request = Zeroizing::new(Vec::with_capacity(2048));
    let mut chunk = Zeroizing::new([0u8; 2048]);
    let mut request_shape: Option<(usize, usize, bool)> = None;
    loop {
        let remaining = request_deadline
            .checked_duration_since(Instant::now())
            .ok_or(CallbackRequestError::BadRequest)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| CallbackRequestError::BadRequest)?;
        let count = stream
            .read(&mut *chunk)
            .map_err(|_| CallbackRequestError::BadRequest)?;
        if count == 0 {
            return Err(CallbackRequestError::BadRequest);
        }
        if request.len().saturating_add(count)
            > MAX_CALLBACK_REQUEST_BYTES + MAX_CALLBACK_BODY_BYTES
        {
            return Err(CallbackRequestError::BadRequest);
        }
        request.extend_from_slice(&chunk[..count]);
        chunk[..count].zeroize();

        if request_shape.is_none() {
            if let Some(header_end) = find_header_end(&request) {
                if header_end > MAX_CALLBACK_REQUEST_BYTES {
                    return Err(CallbackRequestError::BadRequest);
                }
                let (body_length, content_length_present) =
                    declared_body_length(&request[..header_end])?;
                request_shape = Some((header_end, body_length, content_length_present));
            } else if request.len() > MAX_CALLBACK_REQUEST_BYTES {
                return Err(CallbackRequestError::BadRequest);
            }
        }

        if let Some((header_end, body_length, content_length_present)) = request_shape {
            let expected_length = header_end
                .checked_add(body_length)
                .ok_or(CallbackRequestError::BadRequest)?;
            if request.len() > expected_length {
                return Err(CallbackRequestError::BadRequest);
            }
            if request.len() == expected_length {
                return Ok(CallbackHttpRequest {
                    bytes: request,
                    header_end,
                    body_length,
                    content_length_present,
                });
            }
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn declared_body_length(bytes: &[u8]) -> Result<(usize, bool), CallbackRequestError> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    let parsed = request
        .parse(bytes)
        .map_err(|_| CallbackRequestError::BadRequest)?;
    if !parsed.is_complete() {
        return Err(CallbackRequestError::BadRequest);
    }
    if request
        .headers
        .iter()
        .any(|header| header.name.eq_ignore_ascii_case("transfer-encoding"))
    {
        return Err(CallbackRequestError::BadRequest);
    }
    let content_lengths = request
        .headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case("content-length"))
        .collect::<Vec<_>>();
    if content_lengths.len() > 1 {
        return Err(CallbackRequestError::BadRequest);
    }
    let Some(content_length) = content_lengths.first() else {
        return Ok((0, false));
    };
    let value =
        std::str::from_utf8(content_length.value).map_err(|_| CallbackRequestError::BadRequest)?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CallbackRequestError::BadRequest);
    }
    let body_length = value
        .parse::<usize>()
        .map_err(|_| CallbackRequestError::BadRequest)?;
    if body_length > MAX_CALLBACK_BODY_BYTES {
        return Err(CallbackRequestError::BadRequest);
    }
    Ok((body_length, true))
}

fn parse_callback_request(
    callback: &CallbackHttpRequest,
    expected_host: &str,
    start_path: &str,
) -> Result<CallbackAction, CallbackRequestError> {
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut request = httparse::Request::new(&mut headers);
    let parsed = request
        .parse(callback.head())
        .map_err(|_| CallbackRequestError::BadRequest)?;
    if !parsed.is_complete() || request.version != Some(1) {
        return Err(CallbackRequestError::BadRequest);
    }

    let host_headers = request
        .headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case("host"))
        .collect::<Vec<_>>();
    if host_headers.len() != 1
        || !host_headers[0]
            .value
            .eq_ignore_ascii_case(expected_host.as_bytes())
    {
        return Err(CallbackRequestError::BadRequest);
    }

    let method = request.method.ok_or(CallbackRequestError::BadRequest)?;
    let target = request.path.ok_or(CallbackRequestError::BadRequest)?;
    match method {
        "GET" => {
            if callback.body_length != 0 {
                return Err(CallbackRequestError::BadRequest);
            }
            if target == start_path {
                return Ok(CallbackAction::Start);
            }
            if target == COMPLETION_PATH {
                return Err(CallbackRequestError::MethodNotAllowed);
            }
            let Some((path, query)) = target.split_once('?') else {
                return Err(CallbackRequestError::NotFound);
            };
            if path != "/" {
                return Err(CallbackRequestError::NotFound);
            }
            parse_credentials_query(query).map(CallbackAction::Credentials)
        }
        "POST" => {
            if target != COMPLETION_PATH {
                return if target.starts_with("/?") {
                    Err(CallbackRequestError::MethodNotAllowed)
                } else {
                    Err(CallbackRequestError::NotFound)
                };
            }
            if !callback.content_length_present || callback.body_length == 0 {
                return Err(CallbackRequestError::BadRequest);
            }
            let content_types = request
                .headers
                .iter()
                .filter(|header| header.name.eq_ignore_ascii_case("content-type"))
                .collect::<Vec<_>>();
            if content_types.len() != 1
                || !content_types[0]
                    .value
                    .eq_ignore_ascii_case(b"application/x-www-form-urlencoded")
            {
                return Err(CallbackRequestError::BadRequest);
            }
            parse_completion_body(callback.body())
        }
        _ => Err(CallbackRequestError::MethodNotAllowed),
    }
}

fn parse_credentials_query(query: &str) -> Result<BrowserCredentials, CallbackRequestError> {
    if query.is_empty() || query.contains('#') || !has_valid_percent_encoding(query.as_bytes()) {
        return Err(CallbackRequestError::BadRequest);
    }
    let mut access_key_id: Option<Zeroizing<String>> = None;
    let mut secret_access_key: Option<Zeroizing<String>> = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        let value = Zeroizing::new(value.into_owned());
        if value.is_empty()
            || value.len() > MAX_CALLBACK_CREDENTIAL_BYTES
            || value.contains('\u{fffd}')
            || value.chars().any(char::is_control)
        {
            return Err(CallbackRequestError::BadRequest);
        }
        match key.as_ref() {
            "access-key-id" if access_key_id.is_none() && value.trim() == value.as_str() => {
                access_key_id = Some(value);
            }
            "secret-access-key" if secret_access_key.is_none() => {
                secret_access_key = Some(value);
            }
            _ => return Err(CallbackRequestError::BadRequest),
        }
    }
    let access_key_id = access_key_id.ok_or(CallbackRequestError::BadRequest)?;
    let secret_access_key = secret_access_key.ok_or(CallbackRequestError::BadRequest)?;
    Ok(BrowserCredentials {
        access_key_id,
        secret_access_key,
    })
}

fn parse_completion_body(bytes: &[u8]) -> Result<CallbackAction, CallbackRequestError> {
    if bytes.is_empty() || !has_valid_percent_encoding(bytes) {
        return Err(CallbackRequestError::BadRequest);
    }
    let mut handle: Option<Zeroizing<String>> = None;
    let mut flow_id: Option<Zeroizing<String>> = None;
    for (key, value) in url::form_urlencoded::parse(bytes) {
        let value = Zeroizing::new(value.into_owned());
        if !is_flow_token(&value) {
            return Err(CallbackRequestError::BadRequest);
        }
        match key.as_ref() {
            "handle" if handle.is_none() => handle = Some(value),
            "flow" if flow_id.is_none() => flow_id = Some(value),
            _ => return Err(CallbackRequestError::BadRequest),
        }
    }
    Ok(CallbackAction::Complete {
        handle: handle.ok_or(CallbackRequestError::BadRequest)?,
        flow_id: flow_id.ok_or(CallbackRequestError::BadRequest)?,
    })
}

fn is_flow_token(value: &str) -> bool {
    value.len() == FLOW_TOKEN_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn has_valid_percent_encoding(bytes: &[u8]) -> bool {
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

fn start_page(flow_id: &str, broker_url: &Url) -> Zeroizing<String> {
    let flow_id_json = Zeroizing::new(
        serde_json::to_string(flow_id).expect("a generated flow token is JSON encodable"),
    );
    let broker_url_json = Zeroizing::new(
        serde_json::to_string(broker_url.as_str()).expect("a broker URL is JSON encodable"),
    );
    Zeroizing::new(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>Cloudflare authorization</title></head><body><p id=\"status\">Opening Cloudflare authorization…</p><script nonce=\"{flow_id}\">\"use strict\";(() => {{try {{sessionStorage.setItem(\"{FLOW_SESSION_KEY}\", {});location.replace({});}} catch (_) {{document.getElementById(\"status\").textContent = \"Mallard could not start Cloudflare authorization. Return to the terminal and try again.\";}}}})();</script></body></html>",
        flow_id_json.as_str(),
        broker_url_json.as_str(),
    ))
}

fn completion_page(handle: &str) -> Zeroizing<String> {
    let handle_json = Zeroizing::new(
        serde_json::to_string(handle).expect("a generated pending handle is JSON encodable"),
    );
    let welcome_url_json = Zeroizing::new(
        serde_json::to_string(BROKER_WELCOME_URL).expect("the welcome URL is JSON encodable"),
    );
    Zeroizing::new(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>Completing Cloudflare authorization</title></head><body><p id=\"status\">Completing Cloudflare authorization…</p><script nonce=\"{handle}\">\"use strict\";(() => {{history.replaceState(null, \"\", \"{SAFE_CALLBACK_PATH}\");const key = \"{FLOW_SESSION_KEY}\";const flow = sessionStorage.getItem(key);sessionStorage.removeItem(key);const status = document.getElementById(\"status\");if (flow === null) {{status.textContent = \"This Cloudflare authorization did not start in this browser tab. Return to the terminal and try again.\";return;}}const body = new URLSearchParams();body.set(\"handle\", {});body.set(\"flow\", flow);fetch(\"{COMPLETION_PATH}\", {{method: \"POST\",headers: {{\"Content-Type\": \"application/x-www-form-urlencoded\"}},body: body.toString(),cache: \"no-store\",credentials: \"same-origin\",redirect: \"error\",referrerPolicy: \"no-referrer\"}}).then((response) => {{if (!response.ok) throw new Error(\"authorization rejected\");location.replace({});}}).catch(() => {{status.textContent = \"Mallard could not complete Cloudflare authorization. Return to the terminal and try again.\";}});}})();</script></body></html>",
        handle_json.as_str(),
        welcome_url_json.as_str(),
    ))
}

fn write_html_response(
    stream: &mut TcpStream,
    body: &str,
    script_nonce: &str,
    allow_same_origin_connect: bool,
) -> std::io::Result<()> {
    let connect_source = if allow_same_origin_connect {
        "; connect-src 'self'"
    } else {
        ""
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nCache-Control: no-store, max-age=0\r\nPragma: no-cache\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'nonce-{script_nonce}'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'{connect_source}\r\nContent-Type: text/html; charset=utf-8\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body.as_bytes())
}

fn write_http_response(stream: &mut TcpStream, status: u16) -> std::io::Result<()> {
    let reason = match status {
        204 => "No Content",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Bad Request",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nCache-Control: no-store, max-age=0\r\nPragma: no-cache\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn callback_failed_error() -> CliError {
    CliError::remote(
        "r2_authorization_callback_failed",
        "Mallard could not complete the Cloudflare authorization callback.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::sync::mpsc;

    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";

    fn callback_request(query: &str) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(
            format!("GET /?{query} HTTP/1.1\r\nHost: localhost:8976\r\nConnection: close\r\n\r\n")
                .into_bytes(),
        )
    }

    fn assembled_request(
        bytes: Zeroizing<Vec<u8>>,
    ) -> Result<CallbackHttpRequest, CallbackRequestError> {
        let header_end = find_header_end(&bytes).ok_or(CallbackRequestError::BadRequest)?;
        let (body_length, content_length_present) = declared_body_length(&bytes[..header_end])?;
        if bytes.len() != header_end + body_length {
            return Err(CallbackRequestError::BadRequest);
        }
        Ok(CallbackHttpRequest {
            bytes,
            header_end,
            body_length,
            content_length_present,
        })
    }

    fn send_http(address: SocketAddr, request: &[u8]) -> String {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.write_all(request).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    fn between<'a>(text: &'a str, prefix: &str, suffix: &str) -> &'a str {
        let start = text.find(prefix).unwrap() + prefix.len();
        let remainder = &text[start..];
        &remainder[..remainder.find(suffix).unwrap()]
    }

    #[test]
    fn destination_is_stable_canonical_and_name_scoped() {
        let first = derive_destination(ACCOUNT, "R2 storage 1");
        let retry = derive_destination(ACCOUNT, "R2 storage 1");
        let other = derive_destination(ACCOUNT, "R2 storage 2");
        let other_account = derive_destination("fedcba9876543210fedcba9876543210", "R2 storage 1");
        assert_eq!(first.bucket, retry.bucket);
        assert_eq!(first.api_url, retry.api_url);
        assert_ne!(first.bucket, other.bucket);
        assert_ne!(first.bucket, other_account.bucket);
        assert!(first.bucket.starts_with("mallard-r2-storage-1-"));
        let suffix = first.bucket.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), R2_BUCKET_SUFFIX_HEX_BYTES);
        assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            first.api_url,
            format!(
                "https://{ACCOUNT}.r2.cloudflarestorage.com/{}",
                first.bucket
            )
        );
        assert!(first
            .bucket
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'));
    }

    #[test]
    fn destination_uses_a_bounded_readable_slug_with_a_safe_fallback() {
        let readable = derive_destination(ACCOUNT, "Team / Backup -- East!");
        assert!(readable.bucket.starts_with("mallard-team-backup-east-"));

        let long = derive_destination(ACCOUNT, &"Readable storage ".repeat(20));
        assert_eq!(long.bucket.len(), R2_BUCKET_MAX_BYTES);
        assert!(long
            .bucket
            .starts_with("mallard-readable-storage-readable-"));
        assert!(long
            .bucket
            .as_bytes()
            .last()
            .unwrap()
            .is_ascii_alphanumeric());

        let fallback = derive_destination(ACCOUNT, "東京 🚀");
        assert!(fallback.bucket.starts_with("mallard-storage-"));

        let first_collision = derive_destination(ACCOUNT, "Team__Backup");
        let second_collision = derive_destination(ACCOUNT, "team backup");
        assert!(first_collision.bucket.starts_with("mallard-team-backup-"));
        assert!(second_collision.bucket.starts_with("mallard-team-backup-"));
        assert_ne!(first_collision.bucket, second_collision.bucket);

        for destination in [readable, long, fallback, first_collision, second_collision] {
            assert!((3..=R2_BUCKET_MAX_BYTES).contains(&destination.bucket.len()));
            assert!(destination
                .bucket
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'));
            assert!(destination
                .bucket
                .as_bytes()
                .first()
                .unwrap()
                .is_ascii_alphanumeric());
            assert!(destination
                .bucket
                .as_bytes()
                .last()
                .unwrap()
                .is_ascii_alphanumeric());
            let parsed =
                mallard_application::storage_setup::parse_r2_api_url(&destination.api_url).unwrap();
            assert_eq!(parsed.bucket(), destination.bucket);
        }
    }

    #[test]
    fn whoami_accepts_one_account_and_fails_closed_on_ambiguity() {
        let one = format!(
            "{{\"loggedIn\":true,\"email\":\"private@example.test\",\"accounts\":[{{\"name\":\"Private\",\"id\":\"{ACCOUNT}\"}}]}}"
        );
        let AuthState::Authenticated(account_id) = parse_whoami(true, one.as_bytes()).unwrap()
        else {
            panic!("expected an authenticated account");
        };
        assert_eq!(account_id, ACCOUNT);

        let multiple = format!(
            "{{\"loggedIn\":true,\"accounts\":[{{\"id\":\"{ACCOUNT}\"}},{{\"id\":\"ffffffffffffffffffffffffffffffff\"}}]}}"
        );
        let Err(error) = parse_whoami(true, multiple.as_bytes()) else {
            panic!("multiple accounts must fail closed");
        };
        assert_eq!(error.code(), "wrangler_account_ambiguous");
        assert!(!error.to_string().contains(ACCOUNT));
    }

    #[test]
    fn whoami_distinguishes_logged_out_from_untrusted_output() {
        assert!(matches!(
            parse_whoami(false, br#"{"loggedIn":false}"#).unwrap(),
            AuthState::LoggedOut
        ));
        for output in [
            br#"{"loggedIn":true,"accounts":[]}"#.as_slice(),
            br#"{"loggedIn":true,"accounts":[{"id":"not-an-account"}]}"#.as_slice(),
            b"private malformed wrangler output".as_slice(),
        ] {
            let Err(error) = parse_whoami(true, output) else {
                panic!("untrusted Wrangler output must fail closed");
            };
            assert_eq!(error.code(), "wrangler_auth_unavailable");
            assert!(!error.to_string().contains("private"));
        }
    }

    #[test]
    fn wrangler_commands_strip_secret_environment_and_scope_the_account() {
        let client = WranglerClient::with_program(OsString::from("wrangler-test")).unwrap();
        let login_command = client.command(&["login"], None);
        assert_eq!(
            login_command.get_args().collect::<Vec<_>>(),
            ["login"].map(OsStr::new)
        );
        let login_environment = login_command.get_envs().collect::<Vec<_>>();
        for variable in REMOVED_WRANGLER_ENVIRONMENT_VARIABLES {
            assert!(login_environment
                .iter()
                .any(|(name, value)| { *name == OsStr::new(variable) && value.is_none() }));
        }
        for (variable, expected) in [
            ("WRANGLER_SEND_METRICS", "false"),
            ("WRANGLER_SEND_ERROR_REPORTS", "false"),
            ("WRANGLER_NO_SKILLS_UPDATE_PROMPTS", "true"),
            ("WRANGLER_WRITE_LOGS", "false"),
            ("WRANGLER_LOG_SANITIZE", "true"),
        ] {
            assert!(login_environment.iter().any(|(name, value)| {
                *name == OsStr::new(variable) && *value == Some(OsStr::new(expected))
            }));
        }

        let command = client.command(&["r2", "bucket", "info", "mallard-safe"], Some(ACCOUNT));
        let environment = command.get_envs().collect::<Vec<_>>();
        assert!(environment.iter().any(|(name, value)| {
            *name == OsStr::new("CLOUDFLARE_ACCOUNT_ID") && *value == Some(OsStr::new(ACCOUNT))
        }));
        assert!(environment
            .iter()
            .any(|(name, value)| { *name == OsStr::new("CF_ACCOUNT_ID") && value.is_none() }));
        assert_eq!(command.get_program(), OsStr::new("wrangler-test"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["r2", "bucket", "info", "mallard-safe"].map(OsStr::new)
        );
    }

    #[test]
    fn oversized_wrangler_output_is_rejected_by_the_pipe_reader() {
        let output = vec![b'x'; MAX_WRANGLER_JSON_BYTES as usize + 1];
        assert!(matches!(
            read_bounded_stream(std::io::Cursor::new(output)),
            Err(ProcessFailure::OutputTooLarge)
        ));
    }

    #[test]
    fn broker_url_has_only_encoded_public_setup_coordinates() {
        let url = broker_url(ACCOUNT, "mallard-safe");
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("oauth.pipelines.cloudflare.com"));
        assert_eq!(url.path(), "/oauth/login");
        let pairs = url.query_pairs().collect::<Vec<_>>();
        assert_eq!(
            pairs,
            [
                ("accountId".into(), ACCOUNT.into()),
                ("bucketName".into(), "mallard-safe".into()),
                ("pipelineName".into(), "mallard-safe".into()),
            ]
        );
    }

    #[test]
    fn callback_parser_accepts_exact_bounded_credentials() {
        let request =
            callback_request("access-key-id=access-canary&secret-access-key=secret%2Bcanary");
        let request = assembled_request(request)
            .unwrap_or_else(|_| panic!("the complete test request must assemble"));
        let Ok(CallbackAction::Credentials(credentials)) =
            parse_callback_request(&request, BROKER_CALLBACK_ADDRESS, "/unused-start")
        else {
            panic!("the exact callback must be accepted");
        };
        assert_eq!(credentials.access_key_id.as_str(), "access-canary");
        assert_eq!(credentials.secret_access_key.as_str(), "secret+canary");
    }

    #[test]
    fn callback_parser_rejects_ambiguous_or_malformed_requests() {
        for request in [
            callback_request("access-key-id=a&access-key-id=b&secret-access-key=s"),
            callback_request("access-key-id=a"),
            callback_request("access-key-id=a&secret-access-key=%zz"),
            callback_request("access-key-id=a&secret-access-key=s&extra=value"),
            Zeroizing::new(
                b"POST /?access-key-id=a&secret-access-key=s HTTP/1.1\r\nHost: localhost:8976\r\n\r\n"
                    .to_vec(),
            ),
            Zeroizing::new(
                b"GET /other?access-key-id=a&secret-access-key=s HTTP/1.1\r\nHost: localhost:8976\r\n\r\n"
                    .to_vec(),
            ),
        ] {
            let request = assembled_request(request)
                .unwrap_or_else(|_| panic!("the malformed test request must assemble"));
            assert!(parse_callback_request(
                &request,
                BROKER_CALLBACK_ADDRESS,
                "/unused-start"
            )
            .is_err());
        }

        let oversized_secret = "s".repeat(MAX_CALLBACK_CREDENTIAL_BYTES + 1);
        let request = callback_request(&format!(
            "access-key-id=a&secret-access-key={oversized_secret}"
        ));
        let request = assembled_request(request)
            .unwrap_or_else(|_| panic!("the oversized test request must assemble"));
        assert!(
            parse_callback_request(&request, BROKER_CALLBACK_ADDRESS, "/unused-start").is_err()
        );
    }

    #[test]
    fn valid_browser_context_handshake_completes_without_echoing_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = broker_url(ACCOUNT, "mallard-safe");
        let (response_sender, response_receiver) = mpsc::channel();
        let credentials = authorize_with_listener(
            listener,
            &url,
            move |start_url| {
                assert_eq!(start_url.scheme(), "http");
                assert_eq!(start_url.host_str(), Some("localhost"));
                assert_eq!(start_url.port(), Some(address.port()));
                assert!(start_url.path().starts_with(START_PATH_PREFIX));
                let start_path = start_url.path().to_string();
                let flow_id = start_path
                    .strip_prefix(START_PATH_PREFIX)
                    .expect("the start path carries the browser flow")
                    .to_string();
                assert!(is_flow_token(&flow_id));
                thread::spawn(move || {
                    let host = format!("localhost:{}", address.port());
                    let start_request = format!(
                        "GET {start_path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
                    );
                    let start_response = send_http(address, start_request.as_bytes());
                    let credential_request = format!(
                        "GET /?access-key-id=access-canary&secret-access-key=secret-canary HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
                    );
                    let credential_response = send_http(address, credential_request.as_bytes());
                    let handle = between(
                        &credential_response,
                        "body.set(\"handle\", \"",
                        "\");",
                    );
                    assert!(is_flow_token(handle));
                    let completion_body = format!("handle={handle}&flow={flow_id}");
                    let completion_request = format!(
                        "POST {COMPLETION_PATH} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{completion_body}",
                        completion_body.len()
                    );
                    let completion_response = send_http(address, completion_request.as_bytes());
                    response_sender
                        .send((
                            start_response,
                            credential_response,
                            completion_response,
                        ))
                        .unwrap();
                });
                Ok(())
            },
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(credentials.access_key_id.as_str(), "access-canary");
        assert_eq!(credentials.secret_access_key.as_str(), "secret-canary");
        let (start_response, credential_response, completion_response) = response_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(start_response.starts_with("HTTP/1.1 200 OK"));
        assert!(start_response.contains("Cache-Control: no-store"));
        assert!(start_response.contains("sessionStorage.setItem"));
        assert!(start_response.contains(BROKER_URL));
        assert!(credential_response.starts_with("HTTP/1.1 200 OK"));
        assert!(credential_response.contains("history.replaceState"));
        assert!(credential_response.contains("sessionStorage.removeItem"));
        assert!(credential_response.contains("connect-src 'self'"));
        assert!(completion_response.starts_with("HTTP/1.1 204 No Content"));
        for response in [&start_response, &credential_response, &completion_response] {
            assert!(!response.contains("access-canary"));
            assert!(!response.contains("secret-canary"));
        }
    }

    #[test]
    fn raw_injected_callback_canary_cannot_complete_or_reach_output() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = broker_url(ACCOUNT, "mallard-safe");
        let (response_sender, response_receiver) = mpsc::channel();
        let Err(error) = authorize_with_listener(
            listener,
            &url,
            move |_| {
                thread::spawn(move || {
                    let request = format!(
                        "GET /?access-key-id=raw-access-canary&secret-access-key=raw-secret-canary HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\n\r\n",
                        address.port()
                    );
                    response_sender
                        .send(send_http(address, request.as_bytes()))
                        .unwrap();
                });
                Ok(())
            },
            Duration::from_millis(125),
        ) else {
            panic!("a callback without the browser flow must not return credentials");
        };
        assert_eq!(error.code(), "r2_authorization_timeout");
        let response = response_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"));
        for output in [response.as_str(), &error.to_string(), &format!("{error:?}")] {
            assert!(!output.contains("raw-access-canary"));
            assert!(!output.contains("raw-secret-canary"));
        }
    }

    #[test]
    fn wrong_flow_or_handle_fails_and_success_cannot_be_replayed() {
        let mut flow = BrowserAuthorizationFlow::new(
            BROKER_CALLBACK_ADDRESS.to_string(),
            broker_url(ACCOUNT, "mallard-safe"),
        )
        .unwrap();
        let expected_flow = Zeroizing::new(flow.flow_id.to_string());
        let page = flow
            .stage_credentials(BrowserCredentials {
                access_key_id: Zeroizing::new("access-canary".to_string()),
                secret_access_key: Zeroizing::new("secret-canary".to_string()),
            })
            .unwrap();
        let expected_handle = Zeroizing::new(
            flow.pending
                .as_ref()
                .expect("credentials are pending")
                .handle
                .to_string(),
        );
        let mut wrong = Zeroizing::new("0".repeat(FLOW_TOKEN_HEX_BYTES));
        if wrong.as_str() == expected_flow.as_str() || wrong.as_str() == expected_handle.as_str() {
            wrong.replace_range(..1, "1");
        }

        assert!(flow
            .complete(&wrong, &expected_flow, Instant::now())
            .is_none());
        assert!(flow
            .complete(&expected_handle, &wrong, Instant::now())
            .is_none());
        let credentials = flow
            .complete(&expected_handle, &expected_flow, Instant::now())
            .expect("the exact browser flow and handle complete once");
        assert_eq!(credentials.access_key_id.as_str(), "access-canary");
        assert_eq!(credentials.secret_access_key.as_str(), "secret-canary");
        assert!(flow
            .complete(&expected_handle, &expected_flow, Instant::now())
            .is_none());
        assert!(!page.contains("access-canary"));
        assert!(!page.contains("secret-canary"));
    }

    #[test]
    fn reusable_callback_listener_rebinds_after_an_accepted_connection() {
        let first = bind_reusable_listener("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = first.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            let mut byte = [0u8; 1];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });
        let (accepted, _) = first.accept().unwrap();
        drop(accepted);
        client.join().unwrap();
        drop(first);

        let rebound = bind_reusable_listener(address).unwrap();
        assert_eq!(rebound.local_addr().unwrap(), address);
    }

    #[test]
    fn browser_failure_and_timeout_return_only_fixed_errors() {
        let url = broker_url(ACCOUNT, "mallard-safe");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let Err(browser_error) =
            authorize_with_listener(listener, &url, |_| Err(()), Duration::from_millis(20))
        else {
            panic!("browser failure must stop setup");
        };
        assert_eq!(browser_error.code(), "r2_authorization_browser_failed");
        assert!(!browser_error.to_string().contains(ACCOUNT));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let Err(timeout_error) =
            authorize_with_listener(listener, &url, |_| Ok(()), Duration::from_millis(20))
        else {
            panic!("an absent callback must time out");
        };
        assert_eq!(timeout_error.code(), "r2_authorization_timeout");
        assert!(!timeout_error.to_string().contains(ACCOUNT));
    }

    #[test]
    fn callback_deadline_bounds_a_drip_feed_client() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = broker_url(ACCOUNT, "mallard-safe");
        let started = Instant::now();
        let Err(error) = authorize_with_listener(
            listener,
            &url,
            move |_| {
                thread::spawn(move || {
                    let mut stream = TcpStream::connect(address).unwrap();
                    for _ in 0..1_000 {
                        if stream.write_all(b"G").is_err() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                });
                Ok(())
            },
            Duration::from_millis(40),
        ) else {
            panic!("an incomplete callback must time out");
        };
        assert_eq!(error.code(), "r2_authorization_timeout");
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
