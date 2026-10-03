use crate::error::CliError;
use mallard_application::storage_setup::SecretString;
use mallard_domain::{HostId, SecretValue};
use mallard_infrastructure::codex_binary::find_binary;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
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
const MAX_WRANGLER_VERSION_CHARS: usize = 32;
const MAX_WRANGLER_PATH_CHARS: usize = 1024;
const MAX_CALLBACK_REQUEST_BYTES: usize = 16 * 1024;
const MAX_CALLBACK_BODY_BYTES: usize = 512;
const MAX_CALLBACK_CREDENTIAL_BYTES: usize = 4 * 1024;
const MAX_CALLBACK_REQUESTS: usize = 16;
const FLOW_TOKEN_BYTES: usize = 32;
const FLOW_TOKEN_HEX_BYTES: usize = FLOW_TOKEN_BYTES * 2;
const START_PATH_PREFIX: &str = "/mallard/r2/start/";
const COMPLETION_PATH: &str = "/mallard/r2/complete";
const SAFE_CALLBACK_PATH: &str = "/mallard/r2/authorizing";
const FLOW_SESSION_KEY: &str = "mallard:r2:browser-flow";
const WHOAMI_TIMEOUT: Duration = Duration::from_secs(30);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(180);
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
    prefix_args: Vec<OsString>,
    runtime_path: Option<OsString>,
    workdir: TempDir,
}

impl WranglerClient {
    pub fn new() -> Result<Self, CliError> {
        #[cfg(not(windows))]
        let mut client = {
            let program = find_binary("wrangler")
                .map_err(|_| map_wrangler_auth_process_error(ProcessFailure::Missing))?;
            let program = program.canonicalize().map_err(|_| runtime_error())?;
            Self::with_program(program.into_os_string())?
        };
        #[cfg(windows)]
        let mut client = {
            // npm installs a .cmd shim, which cannot be run with an argv-only
            // process API. Run its installed JavaScript entry with Node.
            let entry = std::env::var_os("PATH")
                .into_iter()
                .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
                .map(|directory| directory.join("node_modules/wrangler/bin/wrangler.js"))
                .find(|path| path.is_file())
                .ok_or_else(|| map_wrangler_auth_process_error(ProcessFailure::Missing))?;
            let node = find_binary("node").map_err(|_| runtime_error())?;
            let mut client = Self::with_program(node.into_os_string())?;
            client.prefix_args.push(entry.into_os_string());
            client
        };
        if let Ok(node) = find_binary("node") {
            if let Some(directory) = node.parent() {
                let mut paths = vec![directory.to_path_buf()];
                paths.extend(std::env::split_paths(
                    &std::env::var_os("PATH").unwrap_or_default(),
                ));
                client.runtime_path =
                    Some(std::env::join_paths(paths).map_err(|_| runtime_error())?);
            }
        }
        Ok(client)
    }

    fn with_program(program: OsString) -> Result<Self, CliError> {
        let workdir = tempfile::Builder::new()
            .prefix("mallard-wrangler-")
            .tempdir()
            .map_err(|_| CliError::persistence("Mallard could not prepare Cloudflare setup."))?;
        Ok(Self {
            program,
            prefix_args: Vec::new(),
            runtime_path: None,
            workdir,
        })
    }

    /// Confirm Wrangler starts and supports the commands setup needs, and
    /// say which Wrangler that is: its version and where it was found.
    pub fn check_dependency(&self) -> Result<WranglerInfo, CliError> {
        let (status, output) = self
            .run_json(&["--version"], WHOAMI_TIMEOUT)
            .map_err(|_| runtime_error())?;
        if !status.success() {
            return Err(runtime_error());
        }
        let version = parse_wrangler_version(&output);
        let (status, output) = self
            .run_json(&["auth", "token", "--help"], WHOAMI_TIMEOUT)
            .map_err(|_| runtime_error())?;
        if !status.success() || !String::from_utf8_lossy(&output).contains("--json") {
            return Err(CliError::invalid_code(
                "wrangler_update_required",
                "Update Wrangler to a version with auth token --json support.",
            ));
        }
        let path = self
            .prefix_args
            .first()
            .unwrap_or(&self.program)
            .to_string_lossy()
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_WRANGLER_PATH_CHARS)
            .collect();
        Ok(WranglerInfo { version, path })
    }

    pub fn login(&self) -> Result<(), CliError> {
        if self.run_login().map_err(map_wrangler_auth_process_error)? {
            Ok(())
        } else {
            Err(CliError::remote(
                "wrangler_login_failed",
                "Cloudflare login was not completed.",
            ))
        }
    }

    pub fn authenticated_accounts(&self) -> Result<Vec<WranglerAccount>, CliError> {
        match self.whoami()? {
            AuthState::Authenticated(accounts) => Ok(accounts),
            AuthState::LoggedOut => {
                self.login()?;
                match self.whoami()? {
                    AuthState::Authenticated(accounts) => Ok(accounts),
                    AuthState::LoggedOut => Err(CliError::remote(
                        "wrangler_login_failed",
                        "Cloudflare login was not completed.",
                    )),
                }
            }
        }
    }

    pub fn selected_account_id(&self, selected: Option<&str>) -> Result<String, CliError> {
        select_account(self.authenticated_accounts()?, selected)
    }

    pub fn oauth_token(&self) -> Result<SecretValue, CliError> {
        let (status, output) = self
            .run_json(&["auth", "token", "--json"], WHOAMI_TIMEOUT)
            .map_err(map_wrangler_auth_process_error)?;
        parse_oauth_token(status.success(), &output)
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

    fn run_login(&self) -> Result<bool, ProcessFailure> {
        let mut command = self.command(&["login", "--browser=false"], None);
        command.stdout(Stdio::piped());
        let mut child = command.spawn().map_err(map_spawn_failure)?;
        let stdout = child.stdout.take().ok_or(ProcessFailure::Unavailable)?;
        let reader = thread::spawn(move || forward_login_link(stdout));
        let status = wait_with_timeout(&mut child, LOGIN_TIMEOUT);
        reader.join().map_err(|_| ProcessFailure::Unavailable)??;
        status.map(|status| status.success())
    }

    fn command(&self, args: &[&str], account_id: Option<&str>) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.prefix_args)
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
            .env_remove("NODE_OPTIONS")
            .env_remove("WRANGLER_LOG_SANITIZE");
        if let Some(path) = &self.runtime_path {
            command.env("PATH", path);
        }
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

// Only these structured progress records reach the desktop. Raw Wrangler
// output and credentials stay out of the IPC stream and activity logs.
fn publish_authorization_link(stage: &str, url: &Url) -> std::io::Result<()> {
    publish_progress(&serde_json::json!({
        "schema": 1, "status": "authorization_link", "stage": stage, "url": url.as_str()
    }))
}

/// Which Wrangler passed the dependency check, said as soon as it did.
pub(crate) fn publish_wrangler_ready(info: &WranglerInfo) -> std::io::Result<()> {
    publish_progress(&serde_json::json!({
        "schema": 1, "status": "wrangler_ready", "version": info.version, "path": info.path
    }))
}

/// The bucket a setup will authorize, said as soon as it exists so the desktop
/// can name it — and keep naming it if consent then expires.
pub(crate) fn publish_bucket_ready(bucket: &str) -> std::io::Result<()> {
    publish_progress(&serde_json::json!({
        "schema": 1, "status": "bucket_ready", "bucket": bucket
    }))
}

fn publish_progress(record: &serde_json::Value) -> std::io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, record)?;
    writeln!(stdout)?;
    stdout.flush()
}

/// The first version-shaped token Wrangler prints for `--version`, or nothing:
/// the check does not depend on it, the setup lane only shows it.
fn parse_wrangler_version(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .split_whitespace()
        .find(|token| {
            token.as_bytes().first().is_some_and(u8::is_ascii_digit)
                && token.contains('.')
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        })
        .map(|token| token.chars().take(MAX_WRANGLER_VERSION_CHARS).collect())
        .unwrap_or_default()
}

fn login_link(line: &str) -> Option<Url> {
    let value = line
        .split_whitespace()
        .find(|part| part.starts_with("https://dash.cloudflare.com/oauth2/auth?"))?;
    if value.len() > 8192 {
        return None;
    }
    let url = Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some("dash.cloudflare.com")
        && url.port().is_none()
        && url.path() == "/oauth2/auth"
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

fn forward_login_link(reader: impl Read) -> Result<(), ProcessFailure> {
    let mut reader = BufReader::new(reader.take(MAX_WRANGLER_JSON_BYTES + 1));
    let mut total = 0;
    let mut sent = false;
    loop {
        let mut line = Zeroizing::new(String::new());
        let count = reader
            .read_line(&mut line)
            .map_err(|_| ProcessFailure::Unavailable)?;
        if count == 0 {
            return Ok(());
        }
        total += count as u64;
        if total > MAX_WRANGLER_JSON_BYTES {
            return Err(ProcessFailure::OutputTooLarge);
        }
        if !sent {
            if let Some(url) = login_link(&line) {
                publish_authorization_link("login", &url)
                    .map_err(|_| ProcessFailure::Unavailable)?;
                sent = true;
            }
        }
    }
}

fn runtime_error() -> CliError {
    CliError::invalid_code(
        "wrangler_runtime_unavailable",
        "Wrangler could not start. Check the Node.js and Wrangler installation.",
    )
}

#[derive(Deserialize)]
struct WhoamiDocument {
    #[serde(rename = "loggedIn")]
    logged_in: bool,
    #[serde(default)]
    accounts: Vec<WranglerAccount>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct WranglerAccount {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// Which Wrangler the setup runs: shown on the setup lane's first step.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WranglerInfo {
    pub version: String,
    pub path: String,
}

enum AuthState {
    LoggedOut,
    Authenticated(Vec<WranglerAccount>),
}

enum ProcessFailure {
    Missing,
    TimedOut,
    OutputTooLarge,
    Unavailable,
}

/// A session Wrangler cannot use is a session to replace. With no token it
/// answers `{"loggedIn":false}`; with a token that expired and could not be
/// refreshed without a terminal it answers an error and no document at all.
/// Both send setup through login. Only a successful run whose output is not
/// the document fails closed.
fn parse_whoami(success: bool, bytes: &[u8]) -> Result<AuthState, CliError> {
    let Ok(document) = serde_json::from_slice::<WhoamiDocument>(bytes) else {
        if !success {
            return Ok(AuthState::LoggedOut);
        }
        return Err(CliError::remote(
            "wrangler_auth_unavailable",
            "Wrangler authentication could not be verified.",
        ));
    };
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
    let mut accounts = Vec::new();
    for mut account in document.accounts {
        if account.id.len() != 32 || !account.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CliError::remote(
                "wrangler_auth_unavailable",
                "Wrangler authentication could not be verified.",
            ));
        }
        account.id.make_ascii_lowercase();
        // Display only bounded account labels. Never forward other whoami fields.
        account.name = account
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(64)
            .collect();
        while account.name.len() > 128 {
            account.name.pop();
        }
        if account_ids.insert(account.id.clone()) {
            accounts.push(account);
        }
    }
    Ok(AuthState::Authenticated(accounts))
}

fn select_account(
    accounts: Vec<WranglerAccount>,
    selected: Option<&str>,
) -> Result<String, CliError> {
    if let Some(id) = selected {
        return accounts.into_iter().find(|account| account.id == id)
            .map(|account| account.id)
            .ok_or_else(|| CliError::invalid_code("wrangler_account_unavailable",
                "The selected account is no longer available. Check the Wrangler login and select an account again."));
    }
    if accounts.len() != 1 {
        return Err(CliError::invalid_code(
            "wrangler_account_ambiguous",
            "Select a Cloudflare account before continuing R2 setup.",
        ));
    }
    Ok(accounts.into_iter().next().expect("one account").id)
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

fn parse_oauth_token(success: bool, bytes: &[u8]) -> Result<SecretValue, CliError> {
    #[derive(Deserialize)]
    struct Token {
        #[serde(rename = "type")]
        kind: String,
        token: String,
    }
    impl Drop for Token {
        fn drop(&mut self) {
            self.token.zeroize();
        }
    }
    let invalid = || {
        CliError::invalid_code(
        "wrangler_oauth_token_unavailable",
        "Mallard requires Wrangler with auth token --json support and an active OAuth login. Update Wrangler and log in again.",
    )
    };
    if !success {
        return Err(invalid());
    }
    let token: Token = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if token.kind != "oauth"
        || token.token.is_empty()
        || token.token.len() > 4096
        || !token
            .token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte))
    {
        return Err(invalid());
    }
    SecretValue::parse(&token.token).map_err(|_| invalid())
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
        |url| publish_authorization_link("bucket", url).map_err(|_| ()),
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
    mallard_infrastructure::loopback_browser::set_exclusive_address_use(&socket)?;
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
                        // Browsers may preconnect without sending HTTP yet.
                        // Closing that idle socket lets them reconnect; sending
                        // a 400 poisons the connection they later use to return.
                        Err(CallbackRequestError::NoRequest) => continue,
                        Err(error) => {
                            let _ = write_http_response(&mut stream, error.status_code());
                        }
                    }
                }
                requests += 1;
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
    NoRequest,
    BadRequest,
    MethodNotAllowed,
    NotFound,
}

impl CallbackRequestError {
    fn status_code(self) -> u16 {
        match self {
            Self::NoRequest | Self::BadRequest => 400,
            Self::MethodNotAllowed => 405,
            Self::NotFound => 404,
        }
    }
}

fn read_callback_request(
    stream: &mut TcpStream,
    callback_deadline: Instant,
) -> Result<CallbackHttpRequest, CallbackRequestError> {
    // Accepted sockets inherit nonblocking mode on macOS. The bounded reads
    // below must wait for HTTP bytes instead of rejecting WouldBlock as 400.
    stream
        .set_nonblocking(false)
        .map_err(|_| CallbackRequestError::BadRequest)?;
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
            .ok_or_else(|| {
                if request.is_empty() {
                    CallbackRequestError::NoRequest
                } else {
                    CallbackRequestError::BadRequest
                }
            })?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| CallbackRequestError::BadRequest)?;
        let count = stream
            .read(&mut *chunk)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                    if request.is_empty() {
                        CallbackRequestError::NoRequest
                    } else {
                        CallbackRequestError::BadRequest
                    }
                }
                _ => CallbackRequestError::BadRequest,
            })?;
        if count == 0 {
            return Err(if request.is_empty() {
                CallbackRequestError::NoRequest
            } else {
                CallbackRequestError::BadRequest
            });
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

    fn send_http(address: SocketAddr, request: &[u8], stage: &'static str) -> String {
        let mut stream = TcpStream::connect(address)
            .unwrap_or_else(|error| panic!("{stage}: connect failed: {error}"));
        stream
            .write_all(request)
            .unwrap_or_else(|error| panic!("{stage}: request write failed: {error}"));
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .unwrap_or_else(|error| panic!("{stage}: response read failed: {error}"));
        response
    }

    fn between<'a>(text: &'a str, prefix: &str, suffix: &str) -> &'a str {
        let start = text.find(prefix).unwrap() + prefix.len();
        let remainder = &text[start..];
        &remainder[..remainder.find(suffix).unwrap()]
    }

    #[test]
    fn oauth_token_requires_the_supported_oauth_contract_and_redacts_errors() {
        assert_eq!(
            parse_oauth_token(true, br#"{"type":"oauth","token":"fixture-token"}"#)
                .unwrap()
                .expose(),
            "fixture-token"
        );
        for bytes in [
            br#"{"type":"api_token","token":"secret-canary"}"#.as_slice(),
            br#"{"type":"oauth","token":"secret-canary\n"}"#.as_slice(),
            br#"{"type":"oauth","token":""}"#.as_slice(),
            b"secret-canary".as_slice(),
        ] {
            let error = parse_oauth_token(true, bytes).unwrap_err();
            assert_eq!(error.code(), "wrangler_oauth_token_unavailable");
            assert!(!format!("{error:?}").contains("secret-canary"));
        }
        assert!(parse_oauth_token(false, br#"{"type":"oauth","token":"fixture-token"}"#).is_err());
    }

    #[test]
    fn login_output_accepts_only_the_cloudflare_authorization_link() {
        let expected = "https://dash.cloudflare.com/oauth2/auth?state=test&code_challenge=test";
        assert_eq!(
            login_link(&format!("Visit this link to authenticate: {expected}\n"))
                .unwrap()
                .as_str(),
            expected
        );
        for line in [
            "private-token-canary",
            "https://evil.example/oauth2/auth?state=test",
            "https://dash.cloudflare.com/oauth2/auth?state=test#fragment",
            "https://dash.cloudflare.com/other?state=test",
        ] {
            assert!(login_link(line).is_none());
        }
    }

    #[test]
    fn whoami_lists_accounts_and_setup_requires_an_available_selection() {
        let one = format!(
            r#"{{"loggedIn":true,"email":"private@example.test","accounts":[{{"name":"Test account","id":"{ACCOUNT}"}}]}}"#
        );
        let AuthState::Authenticated(accounts) = parse_whoami(true, one.as_bytes()).unwrap() else {
            panic!("expected accounts")
        };
        assert_eq!(accounts[0].name, "Test account");
        assert!(!serde_json::to_string(&accounts)
            .unwrap()
            .contains("private@example.test"));
        assert_eq!(select_account(accounts, None).unwrap(), ACCOUNT);

        let multiple = format!(
            r#"{{"loggedIn":true,"accounts":[{{"id":"{ACCOUNT}"}},{{"id":"ffffffffffffffffffffffffffffffff"}}]}}"#
        );
        for (selection, expected) in [
            (None, Err("wrangler_account_ambiguous")),
            (Some(ACCOUNT), Ok(ACCOUNT)),
            (
                Some("ffffffffffffffffffffffffffffffff"),
                Ok("ffffffffffffffffffffffffffffffff"),
            ),
            (
                Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"),
                Err("wrangler_account_unavailable"),
            ),
        ] {
            let AuthState::Authenticated(accounts) =
                parse_whoami(true, multiple.as_bytes()).unwrap()
            else {
                panic!("expected accounts")
            };
            assert_eq!(accounts.len(), 2);
            assert_eq!(
                select_account(accounts, selection).map_err(|e| e.code()),
                expected.map(str::to_owned)
            );
        }
    }

    #[test]
    fn account_labels_are_bounded_and_duplicate_ids_are_removed() {
        let value = serde_json::json!({"loggedIn":true,"accounts":[
            {"id":ACCOUNT,"name":format!("\n{}", "界".repeat(200))}, {"id":ACCOUNT,"name":"Duplicate"}
        ]});
        let AuthState::Authenticated(accounts) =
            parse_whoami(true, &serde_json::to_vec(&value).unwrap()).unwrap()
        else {
            panic!("expected accounts")
        };
        assert_eq!(accounts.len(), 1);
        assert!(accounts[0].name.len() <= 128);
        assert!(!accounts[0].name.contains('\n'));
    }

    #[test]
    fn whoami_distinguishes_logged_out_from_untrusted_output() {
        assert!(matches!(
            parse_whoami(false, br#"{"loggedIn":false}"#).unwrap(),
            AuthState::LoggedOut
        ));
        // An expired token that cannot be refreshed without a terminal ends
        // whoami with an error and nothing on stdout: sign in again.
        for output in [
            b"".as_slice(),
            b"private malformed wrangler output".as_slice(),
        ] {
            assert!(matches!(
                parse_whoami(false, output).unwrap(),
                AuthState::LoggedOut
            ));
        }
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

    #[cfg(unix)]
    #[test]
    fn npm_launcher_uses_resolved_node_for_dependency_checks_and_login() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("wrangler");
        let node = directory.path().join("node");
        std::fs::write(&program, "#!/usr/bin/env node\n").unwrap();
        std::fs::write(&node, "#!/bin/sh\nshift\nprintf '%s\\n' \"$*\" >> \"$0.calls\"\ncase \"$*\" in\n  '--version') printf '4.125.0';;\n  'auth token --help') printf '%s' '--json';;\n  'login --browser=false') exit 0;;\n  *) exit 1;;\nesac\n").unwrap();
        for path in [&program, &node] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut client = WranglerClient::with_program(program.clone().into_os_string()).unwrap();
        // A GUI can start with no Node in PATH. Only the resolved runtime is needed.
        client.runtime_path = Some(directory.path().as_os_str().to_owned());
        let info = client.check_dependency().unwrap();
        assert_eq!(info.version, "4.125.0");
        assert_eq!(info.path, program.to_string_lossy());
        client.login().unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("node.calls")).unwrap(),
            "--version\nauth token --help\nlogin --browser=false\n"
        );
        assert!(client
            .command(&["login"], None)
            .get_envs()
            .any(|(key, value)| key == "NODE_OPTIONS" && value.is_none()));
    }

    #[cfg(unix)]
    #[test]
    fn dependency_errors_distinguish_runtime_and_command_support() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("wrangler");
        let client = WranglerClient::with_program(program.clone().into_os_string()).unwrap();
        for (script, expected) in [
            ("#!/bin/sh\nexit 1\n", "wrangler_runtime_unavailable"),
            ("#!/bin/sh\nexit 0\n", "wrangler_update_required"),
        ] {
            std::fs::write(&program, script).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(client.check_dependency().unwrap_err().code(), expected);
        }
        assert_eq!(
            map_wrangler_auth_process_error(ProcessFailure::Missing).code(),
            "wrangler_not_found"
        );
    }

    #[test]
    fn wrangler_version_is_the_first_version_shaped_token_and_stays_bounded() {
        assert_eq!(parse_wrangler_version(b"4.125.0\n"), "4.125.0");
        assert_eq!(
            parse_wrangler_version(
                "\n \u{26C5}\u{FE0F} wrangler 3.114.2\n-------------------\n".as_bytes()
            ),
            "3.114.2"
        );
        assert_eq!(
            parse_wrangler_version(b"4.0.0-beta.3 (dev)"),
            "4.0.0-beta.3"
        );
        assert_eq!(parse_wrangler_version(b"no version here"), "");
        let long = format!("1.{}", "2".repeat(64));
        assert_eq!(
            parse_wrangler_version(long.as_bytes()).chars().count(),
            MAX_WRANGLER_VERSION_CHARS
        );
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
    fn callback_reader_waits_for_delayed_and_fragmented_requests() {
        for split in [0, 20] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (ready, accepted) = mpsc::channel();
            let client = thread::spawn(move || {
                let mut stream = TcpStream::connect(address).unwrap();
                accepted.recv_timeout(Duration::from_secs(2)).unwrap();
                let request =
                    callback_request("access-key-id=access-canary&secret-access-key=secret-canary");
                stream.write_all(&request[..split]).unwrap();
                thread::sleep(Duration::from_millis(50));
                stream.write_all(&request[split..]).unwrap();
            });
            let (mut stream, _) = listener.accept().unwrap();
            // macOS inherits this mode from the nonblocking listener. Set it
            // explicitly so other platforms exercise the same read behavior.
            stream.set_nonblocking(true).unwrap();
            ready.send(()).unwrap();
            let request =
                read_callback_request(&mut stream, Instant::now() + Duration::from_secs(2));
            client.join().unwrap();
            let request = request
                .unwrap_or_else(|_| panic!("a delayed callback must wait for its HTTP bytes"));
            let Ok(CallbackAction::Credentials(credentials)) =
                parse_callback_request(&request, BROKER_CALLBACK_ADDRESS, "/unused-start")
            else {
                panic!("the delayed callback must be accepted");
            };
            assert_eq!(credentials.access_key_id.as_str(), "access-canary");
            assert_eq!(credentials.secret_access_key.as_str(), "secret-canary");
        }
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
    fn idle_connections_close_silently_but_partial_requests_still_fail() {
        for bytes in [b"".as_slice(), b"GET /".as_slice()] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (sender, receiver) = mpsc::channel();
            let result = authorize_with_listener(
                listener,
                &broker_url(ACCOUNT, "mallard-safe"),
                move |_| {
                    let mut stream = TcpStream::connect(address).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    stream.write_all(bytes).unwrap();
                    thread::spawn(move || {
                        let mut response = String::new();
                        stream.read_to_string(&mut response).unwrap();
                        sender.send(response).unwrap();
                    });
                    Ok(())
                },
                Duration::from_millis(500),
            );
            assert!(result.is_err());
            let response = receiver.recv_timeout(Duration::from_secs(2)).unwrap();
            if bytes.is_empty() {
                assert!(
                    response.is_empty(),
                    "an idle connection must receive no HTTP response"
                );
            } else {
                assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
            }
        }
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
                    // An unused browser preconnection is not an HTTP request
                    // and must not exhaust the flow's malformed-request budget.
                    for _ in 0..=MAX_CALLBACK_REQUESTS {
                        let mut idle = TcpStream::connect(address).unwrap();
                        idle.shutdown(std::net::Shutdown::Write).unwrap();
                        let mut response = String::new();
                        idle.read_to_string(&mut response).unwrap();
                        assert!(response.is_empty());
                    }
                    let host = format!("localhost:{}", address.port());
                    let start_request = format!(
                        "GET {start_path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
                    );
                    let start_response = send_http(address, start_request.as_bytes(), "start");
                    let credential_request = format!(
                        "GET /?access-key-id=access-canary&secret-access-key=secret-canary HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
                    );
                    let credential_response =
                        send_http(address, credential_request.as_bytes(), "credentials");
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
                    let completion_response =
                        send_http(address, completion_request.as_bytes(), "completion");
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
            // Allow slow CI scheduling across 17 idle connections and three
            // HTTP requests, each of which can wait for the 25 ms accept poll.
            Duration::from_secs(10),
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
        // The injected request must be accepted and answered before the
        // callback deadline: a connection still in the backlog when the
        // listener drops is reset, and the reader thread panics instead of
        // seeing the 403. The listener polls every 25ms, so leave a margin
        // wide enough for a slow CI scheduler.
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
                        .send(send_http(address, request.as_bytes(), "injected callback"))
                        .unwrap();
                });
                Ok(())
            },
            Duration::from_secs(1),
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
        const CHILD_FLAG: &str = "MALLARD_CALLBACK_REBIND_TEST_CHILD";
        if std::env::var_os(CHILD_FLAG).is_none() {
            // A concurrent subprocess can retain this process's listener until
            // exec, even with CLOEXEC set. Check immediate rebind in a process
            // that runs only this test, without weakening the bind assertion.
            let current = thread::current();
            let test_name = current.name().expect("the test thread has a name");
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
                .env(CHILD_FLAG, "1")
                .output()
                .expect("start the isolated callback listener test");
            assert!(
                output.status.success(),
                "isolated callback listener test failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }
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
