//! Direct desktop entry to the packaged R2 setup CLI.
use mallard_application::storage_setup::begin_storage_draft_operation;
use mallard_domain::{now_secs, StorageSetupDraftId};
use mallard_infrastructure::persistence::LocalStateRepository;
use mallard_infrastructure::r2_setup::discovery::{
    BucketScan, DiscoveredBucket, MarkerDetailCode, MarkerState,
};
use mallard_infrastructure::r2_setup::{destination, propose_bucket_name};
use mallard_infrastructure::storage_providers::r2::{
    provider_key, valid_bucket_name, R2Jurisdiction,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::sync::watch;
use zeroize::Zeroizing;

const MAX_OUTPUT: u64 = 128 * 1024;
const SETUP_TIMEOUT: Duration = Duration::from_secs(600);

type SetupKey = (StorageSetupDraftId, u64);

/// One desktop setup at a time. Keep one early cancellation to cover IPC ordering.
#[derive(Default)]
pub struct R2SetupControl(Mutex<SetupControlState>);

#[derive(Default)]
struct SetupControlState {
    active: Option<(SetupKey, watch::Sender<bool>)>,
    cancelled: Option<SetupKey>,
    scan: Option<SavedScan>,
    scan_sequence: u64,
}

struct SetupRun<'a> {
    control: &'a R2SetupControl,
    cancellation: watch::Receiver<bool>,
}

impl R2SetupControl {
    fn begin(&self, key: SetupKey) -> Result<SetupRun<'_>, R2SetupFailure> {
        let mut state = self.0.lock().map_err(|_| R2SetupFailure::failed())?;
        if state.active.is_some() {
            return Err(R2SetupFailure {
                code: R2SetupErrorCode::SetupBusy,
            });
        }
        if state.cancelled.take().as_ref() == Some(&key) {
            return Err(R2SetupFailure {
                code: R2SetupErrorCode::Cancelled,
            });
        }
        let (sender, cancellation) = watch::channel(false);
        state.active = Some((key, sender));
        Ok(SetupRun {
            control: self,
            cancellation,
        })
    }

    pub(super) fn cancel(
        &self,
        draft_id: StorageSetupDraftId,
        revision: u64,
    ) -> Result<(), R2SetupFailure> {
        let key = (draft_id, revision);
        let mut state = self.0.lock().map_err(|_| R2SetupFailure::failed())?;
        if let Some((active, sender)) = &state.active {
            if active == &key {
                sender.send_replace(true);
            }
        } else {
            state.cancelled = Some(key);
        }
        Ok(())
    }
}

impl Drop for SetupRun<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.control.0.lock() {
            state.active = None;
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum R2SetupErrorCode {
    WranglerNotFound,
    WranglerRuntimeUnavailable,
    WranglerUpdateRequired,
    SignInRequired,
    AuthorizationTimeout,
    AuthorizationUnavailable,
    AccountAmbiguous,
    AccountUnavailable,
    DraftChanged,
    SetupBusy,
    CreationUnknown,
    PermissionDenied,
    R2NotEnabled,
    AuthenticationFailed,
    RateLimited,
    ServiceUnavailable,
    RequestRejected,
    IdentityMismatch,
    BucketNameTaken,
    Cancelled,
    HelperUnavailable,
    SetupTimeout,
    SetupFailed,
}

#[derive(Clone, Debug, Serialize)]
pub struct R2SetupFailure {
    pub code: R2SetupErrorCode,
}

impl R2SetupFailure {
    pub(super) fn failed() -> Self {
        Self {
            code: R2SetupErrorCode::SetupFailed,
        }
    }

    fn from_code(code: &str) -> Self {
        use R2SetupErrorCode::*;
        Self {
            code: match code {
                "wrangler_not_found" => WranglerNotFound,
                "wrangler_runtime_unavailable" => WranglerRuntimeUnavailable,
                "wrangler_update_required" => WranglerUpdateRequired,
                "wrangler_auth_unavailable"
                | "wrangler_oauth_token_unavailable"
                | "wrangler_login_failed"
                | "wrangler_auth_timeout" => SignInRequired,
                "wrangler_account_ambiguous" => AccountAmbiguous,
                "wrangler_account_unavailable" => AccountUnavailable,
                "r2_authorization_timeout" => AuthorizationTimeout,
                "r2_authorization_callback_unavailable"
                | "r2_authorization_browser_failed"
                | "r2_authorization_callback_failed" => AuthorizationUnavailable,
                "storage_setup_busy" => SetupBusy,
                "storage_setup_draft_revision_conflict"
                | "storage_setup_draft_name_conflict"
                | "storage_setup_draft_target_conflict"
                | "storage_setup_draft_not_found"
                | "storage_setup_draft_provider_conflict"
                | "storage_setup_draft_not_r2" => DraftChanged,
                "r2_setup_outcome_unknown" => CreationUnknown,
                "r2_setup_permission_denied" => PermissionDenied,
                "r2_setup_not_enabled" => R2NotEnabled,
                "r2_setup_authentication_failed" => AuthenticationFailed,
                "r2_setup_rate_limited" => RateLimited,
                "r2_setup_unavailable" => ServiceUnavailable,
                "r2_setup_request_rejected" => RequestRejected,
                "r2_setup_identity_mismatch" => IdentityMismatch,
                "r2_bucket_name_taken" => BucketNameTaken,
                "r2_bucket_already_connected" => DraftChanged,
                _ => SetupFailed,
            },
        }
    }
}

/// One progress record the helper prints while it works, forwarded to the
/// setup lane as it arrives: a page the user must visit, or the bucket a
/// setup will authorize, named as soon as it exists.
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum R2SetupProgress {
    WranglerReady { version: String, path: String },
    AuthorizationLink { stage: String, url: String },
    BucketReady { bucket: String },
}

impl R2SetupProgress {
    fn validate(&self) -> bool {
        match self {
            Self::WranglerReady { version, path } => {
                version.len() <= 32
                    && version.bytes().all(|b| b.is_ascii_graphic())
                    && path.chars().count() <= 1024
                    && !path.chars().any(char::is_control)
            }
            Self::AuthorizationLink { stage, url } => valid_authorization_link(stage, url),
            Self::BucketReady { bucket } => valid_bucket_name(bucket),
        }
    }
}

fn valid_authorization_link(stage: &str, url: &str) -> bool {
    {
        if url.len() > 8192 {
            return false;
        }
        let Ok(url) = tauri::Url::parse(url) else {
            return false;
        };
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return false;
        }
        match stage {
            "login" => {
                url.scheme() == "https"
                    && url.host_str() == Some("dash.cloudflare.com")
                    && url.port().is_none()
                    && url.path() == "/oauth2/auth"
            }
            "bucket" => {
                url.scheme() == "http"
                    && url.host_str() == Some("localhost")
                    && url.port() == Some(8976)
                    && url.query().is_none()
                    && url
                        .path()
                        .strip_prefix("/mallard/r2/start/")
                        .is_some_and(|token| {
                            token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
                        })
            }
            _ => false,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct R2Account {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CliSuccess {
    DraftReady {
        schema: u32,
        reconnected: bool,
    },
    Accounts {
        schema: u32,
        accounts: Vec<R2Account>,
    },
    Buckets {
        schema: u32,
        scan: BucketScan,
    },
}

enum R2Action {
    Accounts(bool),
    Create {
        account_id: String,
        jurisdiction: R2Jurisdiction,
        bucket: String,
    },
    Discover(String),
    Existing {
        account_id: String,
        bucket: DiscoveredBucket,
    },
}

struct SavedScan {
    draft_id: StorageSetupDraftId,
    scan_id: String,
    account_id: String,
    created_at: u64,
    buckets: Vec<DiscoveredBucket>,
}

/// Every sentence here is the English the setup screens print; a code
/// travels beside each one, with the values it was built from, for the
/// interface to translate by.
#[derive(Serialize)]
pub struct DiscoveryCandidate {
    candidate_id: String,
    name: String,
    marker: MarkerState,
    marker_detail: Option<String>,
    marker_detail_code: Option<MarkerDetailCode>,
    /// The version a `layout` detail names.
    marker_layout_version: Option<u32>,
    facts: Vec<DiscoveryFact>,
    cautions: Vec<String>,
    /// Each caution's code, in `cautions` order. A bucket has none.
    caution_codes: Vec<&'static str>,
    connected_storage_id: Option<String>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
struct DiscoveryFact {
    /// `jurisdiction` or `created`.
    key: &'static str,
    label: String,
    value: String,
    /// The jurisdiction's identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    value_code: Option<&'static str>,
    /// The creation date in epoch seconds, when it is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<u64>,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
struct DiscoveryIssue {
    /// `unreadable_buckets` (with its count) or `listing_cut`.
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<usize>,
}

#[derive(Serialize)]
pub struct DiscoveryResult {
    scan_id: String,
    scope: String,
    /// `jurisdictions`: the scope is the jurisdictions read, listed below.
    scope_code: &'static str,
    jurisdictions: Vec<&'static str>,
    status: String,
    issues: Vec<String>,
    /// Each issue's code and count, in `issues` order.
    issue_codes: Vec<DiscoveryIssue>,
    candidates: Vec<DiscoveryCandidate>,
}

pub(super) async fn discover(
    control: &R2SetupControl,
    repository: LocalStateRepository,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    account_id: String,
    on_progress: tauri::ipc::Channel<R2SetupProgress>,
) -> Result<DiscoveryResult, R2SetupFailure> {
    if !valid_account_id(&account_id) {
        return Err(R2SetupFailure::failed());
    }
    control.0.lock().map_err(|_| R2SetupFailure::failed())?.scan = None;
    let result = execute(
        control,
        repository.clone(),
        draft_id.clone(),
        expected_revision,
        R2Action::Discover(account_id.clone()),
        on_progress,
    )
    .await?;
    let CliSuccess::Buckets { scan, .. } = result else {
        return Err(R2SetupFailure::failed());
    };
    if !scan.validate(&account_id) {
        return Err(R2SetupFailure::failed());
    }
    let config = repository
        .load_config()
        .map_err(|_| R2SetupFailure::failed())?;
    let mut state = control.0.lock().map_err(|_| R2SetupFailure::failed())?;
    state.scan_sequence = state
        .scan_sequence
        .checked_add(1)
        .ok_or_else(R2SetupFailure::failed)?;
    let scan_id = state.scan_sequence.to_string();
    let mut candidates = Vec::with_capacity(scan.buckets.len());
    for (index, bucket) in scan.buckets.iter().enumerate() {
        let jurisdiction =
            R2Jurisdiction::parse(&bucket.jurisdiction).ok_or_else(R2SetupFailure::failed)?;
        let api_url = destination(&account_id, jurisdiction, &bucket.name)
            .map_err(|_| R2SetupFailure::failed())?
            .api_url()
            .to_string();
        let detail = bucket.marker_detail.as_ref();
        candidates.push(DiscoveryCandidate {
            candidate_id: index.to_string(),
            name: bucket.name.clone(),
            marker: bucket.marker.clone(),
            marker_detail: detail.map(|detail| detail.text.clone()),
            marker_detail_code: detail.map(|detail| detail.code),
            marker_layout_version: detail.and_then(|detail| detail.layout_version),
            facts: bucket_facts(jurisdiction, &bucket.creation_date),
            cautions: vec![],
            caution_codes: vec![],
            connected_storage_id: config
                .storages
                .iter()
                .find(|storage| {
                    storage.provider == provider_key()
                        && storage.provider_config.settings.get_str("api_url")
                            == Some(api_url.as_str())
                })
                .map(|storage| storage.id.to_string()),
        });
    }
    let unreadable = scan
        .buckets
        .iter()
        .filter(|bucket| bucket.marker == MarkerState::Unreadable)
        .count();
    let (issues, issue_codes) = scan_issues(unreadable, scan.cut);
    let jurisdictions = known_jurisdictions(&scan.jurisdictions);
    state.scan = Some(SavedScan {
        draft_id,
        scan_id: scan_id.clone(),
        account_id: account_id.clone(),
        created_at: now_secs(),
        buckets: scan.buckets,
    });
    Ok(DiscoveryResult {
        scan_id,
        scope: jurisdiction_phrase(&jurisdictions),
        scope_code: "jurisdictions",
        jurisdictions: jurisdictions.iter().map(|value| value.as_str()).collect(),
        status: if scan.partial { "partial" } else { "complete" }.into(),
        issues,
        issue_codes,
        candidates,
    })
}

/// A bucket's row facts: where its data lives and when it was created.
fn bucket_facts(jurisdiction: R2Jurisdiction, creation_date: &str) -> Vec<DiscoveryFact> {
    let created = chrono::DateTime::parse_from_rfc3339(creation_date).ok();
    vec![
        DiscoveryFact {
            key: "jurisdiction",
            label: "Jurisdiction".into(),
            value: jurisdiction.label().into(),
            value_code: Some(jurisdiction.as_str()),
            at: None,
        },
        DiscoveryFact {
            key: "created",
            label: "Created".into(),
            // The date as the row prints it; the raw value when it is not one.
            value: created.map_or_else(
                || creation_date.to_string(),
                |date| date.format("%b %-d, %Y").to_string(),
            ),
            value_code: None,
            at: created.and_then(|date| u64::try_from(date.timestamp()).ok()),
        },
    ]
}

/// What a partial scan skipped, as the notice prints it and by code.
fn scan_issues(unreadable: usize, cut: bool) -> (Vec<String>, Vec<DiscoveryIssue>) {
    let mut issues = Vec::new();
    let mut codes = Vec::new();
    if unreadable > 0 {
        issues.push(format!(
            "{unreadable} bucket{} could not be checked.",
            if unreadable == 1 { "" } else { "s" }
        ));
        codes.push(DiscoveryIssue {
            code: "unreadable_buckets",
            count: Some(unreadable),
        });
    }
    if cut {
        issues.push("Not every bucket could be listed.".into());
        codes.push(DiscoveryIssue {
            code: "listing_cut",
            count: None,
        });
    }
    (issues, codes)
}

/// The jurisdictions a scan read, in its order.
fn known_jurisdictions(values: &[String]) -> Vec<R2Jurisdiction> {
    values
        .iter()
        .filter_map(|value| R2Jurisdiction::parse(value))
        .collect()
}

/// "Default, EU, US and FedRAMP".
fn jurisdiction_phrase(jurisdictions: &[R2Jurisdiction]) -> String {
    let labels: Vec<&str> = jurisdictions.iter().map(|value| value.label()).collect();
    match labels.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

pub(super) async fn prepare_existing(
    control: &R2SetupControl,
    repository: LocalStateRepository,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    scan_id: String,
    candidate_id: String,
    on_progress: tauri::ipc::Channel<R2SetupProgress>,
) -> Result<(), R2SetupFailure> {
    let (account_id, bucket) = {
        let state = control.0.lock().map_err(|_| R2SetupFailure::failed())?;
        selected_bucket(state.scan.as_ref(), &draft_id, &scan_id, &candidate_id)?
    };
    match execute(
        control,
        repository,
        draft_id,
        expected_revision,
        R2Action::Existing { account_id, bucket },
        on_progress,
    )
    .await?
    {
        CliSuccess::DraftReady { .. } => Ok(()),
        _ => Err(R2SetupFailure::failed()),
    }
}

fn selected_bucket(
    scan: Option<&SavedScan>,
    draft_id: &StorageSetupDraftId,
    scan_id: &str,
    candidate_id: &str,
) -> Result<(String, DiscoveredBucket), R2SetupFailure> {
    let stale = || R2SetupFailure {
        code: R2SetupErrorCode::DraftChanged,
    };
    let scan = scan
        .filter(|scan| {
            &scan.draft_id == draft_id
                && scan.scan_id == scan_id
                && now_secs().saturating_sub(scan.created_at) <= 600
        })
        .ok_or_else(stale)?;
    let index: usize = candidate_id.parse().map_err(|_| stale())?;
    let bucket = scan
        .buckets
        .get(index)
        .filter(|bucket| bucket.marker == MarkerState::Storage)
        .ok_or_else(stale)?;
    Ok((scan.account_id.clone(), bucket.clone()))
}

pub(super) async fn accounts(
    control: &R2SetupControl,
    repository: LocalStateRepository,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    reauthenticate: bool,
    on_progress: tauri::ipc::Channel<R2SetupProgress>,
) -> Result<Vec<R2Account>, R2SetupFailure> {
    control.0.lock().map_err(|_| R2SetupFailure::failed())?.scan = None;
    match execute(
        control,
        repository,
        draft_id,
        expected_revision,
        R2Action::Accounts(reauthenticate),
        on_progress,
    )
    .await?
    {
        CliSuccess::Accounts { accounts, .. } => Ok(accounts),
        _ => Err(R2SetupFailure::failed()),
    }
}

pub(super) async fn setup(
    control: &R2SetupControl,
    repository: LocalStateRepository,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    account_id: String,
    jurisdiction: String,
    bucket: String,
    on_progress: tauri::ipc::Channel<R2SetupProgress>,
) -> Result<(), R2SetupFailure> {
    let action = create_action(account_id, &jurisdiction, bucket)?;
    match execute(
        control,
        repository,
        draft_id,
        expected_revision,
        action,
        on_progress,
    )
    .await?
    {
        CliSuccess::DraftReady { .. } => Ok(()),
        _ => Err(R2SetupFailure::failed()),
    }
}

/// The create request, checked as the helper will check it: the account id,
/// the jurisdiction, and the bucket name under Cloudflare's rule.
fn create_action(
    account_id: String,
    jurisdiction: &str,
    bucket: String,
) -> Result<R2Action, R2SetupFailure> {
    if !valid_account_id(&account_id) || !valid_bucket_name(&bucket) {
        return Err(R2SetupFailure::failed());
    }
    let jurisdiction = R2Jurisdiction::parse(jurisdiction).ok_or_else(R2SetupFailure::failed)?;
    Ok(R2Action::Create {
        account_id,
        jurisdiction,
        bucket,
    })
}

/// A readable name for the Bucket line to propose. Whether it is free is
/// only known at creation.
pub(super) fn propose() -> Result<String, R2SetupFailure> {
    propose_bucket_name().map_err(|_| R2SetupFailure::failed())
}

fn valid_account_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn execute(
    control: &R2SetupControl,
    repository: LocalStateRepository,
    draft_id: StorageSetupDraftId,
    expected_revision: u64,
    action: R2Action,
    on_progress: tauri::ipc::Channel<R2SetupProgress>,
) -> Result<CliSuccess, R2SetupFailure> {
    let run = control.begin((draft_id.clone(), expected_revision))?;
    let (program, args) = tauri::async_runtime::spawn_blocking(move || {
        let mut args = setup_args(&repository, &draft_id, expected_revision)?;
        match action {
            R2Action::Accounts(login) => {
                args = vec![
                    "--json".into(),
                    "storage".into(),
                    "r2".into(),
                    "accounts".into(),
                ];
                if login {
                    args.push("--login".into());
                }
            }
            R2Action::Create {
                account_id,
                jurisdiction,
                bucket,
            } => args.extend([
                "--account-id".into(),
                account_id.into(),
                "--jurisdiction".into(),
                jurisdiction.as_str().into(),
                "--bucket".into(),
                bucket.into(),
            ]),
            R2Action::Discover(account_id) => {
                args = vec![
                    "--json".into(),
                    "storage".into(),
                    "r2".into(),
                    "discover".into(),
                    "--account-id".into(),
                    account_id.into(),
                ];
            }
            R2Action::Existing { account_id, bucket } => args.extend([
                "--account-id".into(),
                account_id.into(),
                "--jurisdiction".into(),
                bucket.jurisdiction.into(),
                "--existing-bucket".into(),
                bucket.name.into(),
                "--creation-date".into(),
                bucket.creation_date.into(),
            ]),
        }
        let current = std::env::current_exe().map_err(|_| R2SetupFailure::failed())?;
        let program = super::service::packaged_cli_path(&current).map_err(|_| R2SetupFailure {
            code: R2SetupErrorCode::HelperUnavailable,
        })?;
        Ok::<_, R2SetupFailure>((program, args))
    })
    .await
    .map_err(|_| R2SetupFailure::failed())??;
    run_cli(
        &program,
        &args,
        SETUP_TIMEOUT,
        run.cancellation.clone(),
        Some(&on_progress),
    )
    .await
}

fn setup_args(
    repository: &LocalStateRepository,
    draft_id: &StorageSetupDraftId,
    expected_revision: u64,
) -> Result<Vec<OsString>, R2SetupFailure> {
    let changed = || R2SetupFailure {
        code: R2SetupErrorCode::DraftChanged,
    };
    let draft = repository
        .load_storage_setup_draft(draft_id)
        .map_err(|_| R2SetupFailure::failed())?
        .ok_or_else(changed)?;
    if draft.storage.provider != provider_key() {
        return Err(changed());
    }
    let operation =
        begin_storage_draft_operation(repository, draft_id, expected_revision, &draft.storage.name)
            .map_err(|error| R2SetupFailure::from_code(error.code()))?;
    // The child takes this lock again and checks the same revision.
    drop(operation);
    let args = vec![
        "--json".into(),
        "storage".into(),
        "r2".into(),
        "setup".into(),
        "--draft-id".into(),
        draft_id.as_str().into(),
        "--expected-revision".into(),
        expected_revision.to_string().into(),
        "--name".into(),
        draft.storage.name.into(),
    ];
    Ok(args)
}

async fn read_output(reader: impl AsyncRead + Unpin) -> Result<Zeroizing<Vec<u8>>, R2SetupFailure> {
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take(MAX_OUTPUT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| R2SetupFailure::failed())?;
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err(R2SetupFailure::failed());
    }
    Ok(bytes)
}

async fn read_stdout(
    reader: impl AsyncRead + Unpin,
    mut on_progress: impl FnMut(R2SetupProgress) -> Result<(), R2SetupFailure>,
) -> Result<Zeroizing<Vec<u8>>, R2SetupFailure> {
    let mut reader = BufReader::new(reader.take(MAX_OUTPUT * 4 + 1));
    let mut final_output = Zeroizing::new(Vec::new());
    let mut total = 0;
    let mut records = 0;
    loop {
        let mut line = Zeroizing::new(Vec::new());
        let count = reader
            .read_until(b'\n', &mut line)
            .await
            .map_err(|_| R2SetupFailure::failed())?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_OUTPUT * 4 {
            return Err(R2SetupFailure::failed());
        }
        let value: serde_json::Value =
            serde_json::from_slice(&line).unwrap_or(serde_json::Value::Null);
        if matches!(
            value["status"].as_str(),
            Some("wrangler_ready" | "authorization_link" | "bucket_ready")
        ) {
            if value["schema"] != 1 || records >= 6 {
                return Err(R2SetupFailure::failed());
            }
            let progress: R2SetupProgress =
                serde_json::from_value(value).map_err(|_| R2SetupFailure::failed())?;
            if !progress.validate() {
                return Err(R2SetupFailure::failed());
            }
            records += 1;
            on_progress(progress)?;
        } else {
            if final_output.len() + line.len() > MAX_OUTPUT as usize {
                return Err(R2SetupFailure::failed());
            }
            final_output.extend_from_slice(&line);
        }
    }
    Ok(final_output)
}

async fn run_cli(
    program: &Path,
    args: &[OsString],
    timeout: Duration,
    mut cancellation: watch::Receiver<bool>,
    on_progress: Option<&tauri::ipc::Channel<R2SetupProgress>>,
) -> Result<CliSuccess, R2SetupFailure> {
    let cancelled = || R2SetupFailure {
        code: R2SetupErrorCode::Cancelled,
    };
    if *cancellation.borrow() {
        return Err(cancelled());
    }
    let directory = tempfile::tempdir().map_err(|_| R2SetupFailure::failed())?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|_| R2SetupFailure {
        code: R2SetupErrorCode::HelperUnavailable,
    })?;
    let process_id = child.id();
    let stdout = child.stdout.take().ok_or_else(R2SetupFailure::failed)?;
    let stderr = child.stderr.take().ok_or_else(R2SetupFailure::failed)?;
    let result = tokio::select! {
        biased;
        _ = async {
            while !*cancellation.borrow_and_update() {
                if cancellation.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        } => Err(cancelled()),
        result = tokio::time::timeout(timeout, async {
        tokio::try_join!(read_stdout(stdout, |progress| {
            on_progress.map_or(Ok(()), |channel| channel.send(progress).map_err(|_| R2SetupFailure::failed()))
        }), read_output(stderr), async {
            child.wait().await.map_err(|_| R2SetupFailure::failed())
        },)
        }) => result.map_err(|_| R2SetupFailure { code: R2SetupErrorCode::SetupTimeout }),
    };
    match result {
        Ok(Ok((stdout, stderr, status))) => parse_result(status.code(), &stdout, &stderr),
        Ok(Err(error)) | Err(error) => {
            stop_process_tree(&mut child, process_id).await;
            Err(error)
        }
    }
}

async fn stop_process_tree(child: &mut tokio::process::Child, process_id: Option<u32>) {
    if let Some(pid) = process_id {
        #[cfg(unix)]
        // The helper leads its own process group; Wrangler inherits that group.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let taskkill = std::env::var_os("SystemRoot")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| "C:\\Windows".into())
                .join("System32/taskkill.exe");
            let mut command = tokio::process::Command::new(taskkill);
            command
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(0x08000000)
                .kill_on_drop(true);
            let _ = tokio::time::timeout(Duration::from_secs(5), command.status()).await;
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn parse_result(
    exit: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<CliSuccess, R2SetupFailure> {
    #[derive(Deserialize)]
    struct Failure {
        schema: u32,
        status: String,
        exit_code: i32,
        code: String,
    }
    if exit == Some(0) {
        let result: CliSuccess =
            serde_json::from_slice(stdout).map_err(|_| R2SetupFailure::failed())?;
        match &result {
            CliSuccess::DraftReady {
                schema: 1,
                reconnected,
            } => {
                let _ = reconnected;
                return Ok(result);
            }
            CliSuccess::Accounts {
                schema: 1,
                accounts,
            } if !accounts.is_empty()
                && accounts.len() <= 64
                && accounts.iter().all(|a| {
                    valid_account_id(&a.id)
                        && a.name.chars().count() <= 64
                        && a.name.len() <= 128
                        && !a.name.chars().any(char::is_control)
                })
                && accounts
                    .iter()
                    .map(|a| &a.id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == accounts.len() =>
            {
                return Ok(result)
            }
            CliSuccess::Buckets { schema: 1, scan } if scan.validate(&"0".repeat(32)) => {
                return Ok(result)
            }
            _ => {}
        }
    } else if let Ok(error) = serde_json::from_slice::<Failure>(stderr) {
        if error.schema == 1
            && error.status == "error"
            && Some(error.exit_code) == exit
            && (2..=6).contains(&error.exit_code)
        {
            return Err(R2SetupFailure::from_code(&error.code));
        }
    }
    Err(R2SetupFailure::failed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_application::storage_setup::create_storage_setup_draft;
    use mallard_infrastructure::r2_setup::discovery::MarkerDetail;
    use mallard_infrastructure::storage_providers::StorageProviderRegistry;

    #[test]
    fn existing_bucket_selection_rejects_stale_foreign_and_unrecognized_results() {
        let home = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(home.path()).unwrap();
        let registry = StorageProviderRegistry::production();
        let draft = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
        let other = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
        let mut scan = SavedScan {
            draft_id: draft.draft_id.clone(),
            scan_id: "scan-1".into(),
            account_id: "1".repeat(32),
            created_at: now_secs(),
            buckets: vec![DiscoveredBucket {
                name: "known-bucket".into(),
                creation_date: "2026-09-10T00:00:00Z".into(),
                jurisdiction: "eu".into(),
                marker: MarkerState::Storage,
                marker_detail: Some(MarkerDetail::layout(1)),
            }],
        };
        assert!(selected_bucket(Some(&scan), &draft.draft_id, "scan-1", "0").is_ok());
        assert!(selected_bucket(Some(&scan), &other.draft_id, "scan-1", "0").is_err());
        assert!(selected_bucket(Some(&scan), &draft.draft_id, "scan-2", "0").is_err());
        assert!(selected_bucket(Some(&scan), &draft.draft_id, "scan-1", "1").is_err());
        scan.buckets[0].marker = MarkerState::Missing;
        assert!(selected_bucket(Some(&scan), &draft.draft_id, "scan-1", "0").is_err());
        scan.buckets[0].marker = MarkerState::Storage;
        scan.created_at = now_secs() - 601;
        assert!(selected_bucket(Some(&scan), &draft.draft_id, "scan-1", "0").is_err());
    }

    #[test]
    fn validates_the_exact_r2_draft_and_releases_its_lock_before_launch() {
        let home = tempfile::tempdir().unwrap();
        let repository = LocalStateRepository::from_home_dir(home.path()).unwrap();
        let registry = StorageProviderRegistry::production();
        let draft = create_storage_setup_draft(&repository, registry, &provider_key()).unwrap();
        let args = setup_args(&repository, &draft.draft_id, draft.revision).unwrap();
        assert_eq!(args[9], draft.name.as_str());
        assert_eq!(args.len(), 10);
        assert!(repository.acquire_host_operation().is_ok());
        assert_eq!(
            setup_args(&repository, &draft.draft_id, draft.revision + 1)
                .unwrap_err()
                .code,
            R2SetupErrorCode::DraftChanged
        );
        let local = create_storage_setup_draft(
            &repository,
            registry,
            &mallard_infrastructure::storage_providers::local::provider_key(),
        )
        .unwrap();
        assert_eq!(
            setup_args(&repository, &local.draft_id, local.revision)
                .unwrap_err()
                .code,
            R2SetupErrorCode::DraftChanged
        );
        assert!(repository.load_config().unwrap().storages.is_empty());
    }

    #[test]
    fn a_create_request_is_checked_before_the_helper_is_launched() {
        let account = "0".repeat(32);
        let action = create_action(account.clone(), "eu", "team-backups".into()).unwrap();
        assert!(matches!(
            &action,
            R2Action::Create { account_id, jurisdiction: R2Jurisdiction::Eu, bucket }
                if account_id == &account && bucket == "team-backups"
        ));
        let long = "x".repeat(64);
        for bucket in [
            "Team-Backups",
            "ab",
            "-team",
            "team-",
            long.as_str(),
            "team_backups",
            "",
        ] {
            assert!(
                create_action(account.clone(), "default", bucket.into()).is_err(),
                "{bucket}"
            );
        }
        assert!(create_action("abc".into(), "default", "team-backups".into()).is_err());
        assert!(create_action(account, "fedramp-high", "team-backups".into()).is_err());
        let proposal = propose().unwrap();
        assert!(proposal.starts_with("mallard-"));
        assert!(valid_bucket_name(&proposal));
    }

    #[tokio::test]
    async fn authorization_link_arrives_before_the_helper_finishes() {
        use tokio::io::AsyncWriteExt;
        let (reader, mut writer) = tokio::io::duplex(4096);
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            read_stdout(reader, |progress| {
                sender.send(progress).unwrap();
                Ok(())
            })
            .await
        });
        let wrangler = serde_json::json!({"schema":1,"status":"wrangler_ready","version":"4.125.0","path":"/opt/homebrew/bin/wrangler"});
        writer
            .write_all(format!("{wrangler}\n").as_bytes())
            .await
            .unwrap();
        let ready = serde_json::json!({"schema":1,"status":"bucket_ready","bucket":"mallard-fox"});
        writer
            .write_all(format!("{ready}\n").as_bytes())
            .await
            .unwrap();
        let url = format!("http://localhost:8976/mallard/r2/start/{}", "a".repeat(64));
        let progress = serde_json::json!({"schema":1,"status":"authorization_link","stage":"bucket","url":url});
        writer
            .write_all(format!("{progress}\n").as_bytes())
            .await
            .unwrap();
        let wrangler = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(wrangler, R2SetupProgress::WranglerReady { version, .. } if version == "4.125.0")
        );
        let bucket = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(bucket, R2SetupProgress::BucketReady { bucket } if bucket == "mallard-fox")
        );
        let link = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(link, R2SetupProgress::AuthorizationLink { stage, url: sent } if stage == "bucket" && sent == url)
        );
        assert!(
            !task.is_finished(),
            "link must arrive while authorization is pending"
        );
        writer
            .write_all(b"{\"schema\":1,\"status\":\"draft_ready\",\"reconnected\":false}\n")
            .await
            .unwrap();
        drop(writer);
        let output = task.await.unwrap().unwrap();
        assert!(!String::from_utf8_lossy(&output).contains("/mallard/r2/start/"));
        assert!(matches!(
            parse_result(Some(0), &output, b""),
            Ok(CliSuccess::DraftReady { .. })
        ));
    }

    #[tokio::test]
    async fn authorization_progress_rejects_untrusted_urls_and_excess_output() {
        for (stage, url) in [
            ("login", "https://evil.example/oauth2/auth"),
            (
                "login",
                "https://dash.cloudflare.com.evil.example/oauth2/auth",
            ),
            ("login", "https://user@dash.cloudflare.com/oauth2/auth"),
            ("bucket", "http://localhost:8976/other"),
            ("bucket", "http://remote.example:8976/mallard/r2/start/abc"),
            ("unknown", "https://dash.cloudflare.com/oauth2/auth"),
        ] {
            let progress = serde_json::json!({"schema":1,"status":"authorization_link","stage":stage,"url":url}).to_string();
            assert!(
                read_stdout(progress.as_bytes(), |_| panic!("untrusted link escaped"))
                    .await
                    .is_err()
            );
        }
        for bucket in ["Mallard", "a", "../x", "x".repeat(64).as_str()] {
            let progress =
                serde_json::json!({"schema":1,"status":"bucket_ready","bucket":bucket}).to_string();
            assert!(
                read_stdout(progress.as_bytes(), |_| panic!("invalid bucket escaped"))
                    .await
                    .is_err()
            );
        }
        for (version, path) in [
            ("4.125.0 \n", "/x"),
            ("x".repeat(33).as_str(), "/x"),
            ("4.125.0", "/x\u{7}"),
            ("4.125.0", "y".repeat(1025).as_str()),
        ] {
            let progress = serde_json::json!({"schema":1,"status":"wrangler_ready","version":version,"path":path}).to_string();
            assert!(read_stdout(progress.as_bytes(), |_| panic!(
                "invalid wrangler fact escaped"
            ))
            .await
            .is_err());
        }
        assert!(read_stdout(
            vec![b'x'; (MAX_OUTPUT * 4 + 1) as usize].as_slice(),
            |_| Ok(())
        )
        .await
        .is_err());
    }

    #[test]
    fn account_results_are_bounded_and_do_not_accept_a_setup_result_as_accounts() {
        let valid = serde_json::json!({"schema":1,"status":"accounts","accounts":[{"id":"1".repeat(32),"name":"Team"}]});
        assert!(matches!(
            parse_result(Some(0), &serde_json::to_vec(&valid).unwrap(), b""),
            Ok(CliSuccess::Accounts { .. })
        ));
        for accounts in [
            serde_json::json!([]),
            serde_json::json!([{"id":"invalid","name":"Team"}]),
            serde_json::json!([{"id":"1".repeat(32),"name":"x".repeat(129)}]),
            serde_json::json!([{"id":"1".repeat(32),"name":"Team\n"}]),
            serde_json::json!([{"id":"1".repeat(32),"name":"Team"},{"id":"1".repeat(32),"name":"Duplicate"}]),
        ] {
            let value = serde_json::json!({"schema":1,"status":"accounts","accounts":accounts});
            assert_eq!(
                parse_result(Some(0), &serde_json::to_vec(&value).unwrap(), b"")
                    .unwrap_err()
                    .code,
                R2SetupErrorCode::SetupFailed
            );
        }
    }

    #[test]
    fn scan_results_become_rows_with_jurisdiction_and_creation_facts() {
        let all =
            known_jurisdictions(&["default".into(), "eu".into(), "us".into(), "fedramp".into()]);
        assert_eq!(jurisdiction_phrase(&all), "Default, EU, US and FedRAMP");
        assert_eq!(
            jurisdiction_phrase(&known_jurisdictions(&["default".into(), "eu".into()])),
            "Default and EU"
        );
        assert_eq!(
            jurisdiction_phrase(&known_jurisdictions(&["default".into()])),
            "Default"
        );
        // The English stays as printed; the code and the epoch seconds travel
        // beside it for the interface to word and format.
        assert_eq!(
            bucket_facts(R2Jurisdiction::Eu, "2026-09-02T10:11:12.123Z"),
            [
                DiscoveryFact {
                    key: "jurisdiction",
                    label: "Jurisdiction".into(),
                    value: "EU".into(),
                    value_code: Some("eu"),
                    at: None,
                },
                DiscoveryFact {
                    key: "created",
                    label: "Created".into(),
                    value: "Sep 2, 2026".into(),
                    value_code: None,
                    at: Some(1_788_343_872),
                },
            ]
        );
        let raw = &bucket_facts(R2Jurisdiction::Default, "not a date")[1];
        assert_eq!((raw.value.as_str(), raw.at), ("not a date", None));
        assert_eq!(
            scan_issues(1, true),
            (
                vec![
                    "1 bucket could not be checked.".to_string(),
                    "Not every bucket could be listed.".to_string(),
                ],
                vec![
                    DiscoveryIssue {
                        code: "unreadable_buckets",
                        count: Some(1),
                    },
                    DiscoveryIssue {
                        code: "listing_cut",
                        count: None,
                    },
                ],
            )
        );
        assert_eq!(scan_issues(3, false).0, ["3 buckets could not be checked."]);
        assert_eq!(scan_issues(0, false), (vec![], vec![]));
    }

    #[test]
    fn cli_r2_errors_reach_the_frontend_without_provider_text() {
        for (code, expected) in [
            ("r2_setup_not_enabled", "r2_not_enabled"),
            ("r2_setup_authentication_failed", "authentication_failed"),
            ("r2_setup_permission_denied", "permission_denied"),
            ("r2_setup_rate_limited", "rate_limited"),
            ("r2_setup_unavailable", "service_unavailable"),
            ("r2_setup_request_rejected", "request_rejected"),
            ("r2_setup_outcome_unknown", "creation_unknown"),
        ] {
            let failure = serde_json::json!({"schema":1,"status":"error","exit_code":2,"code":code,"message":"private-canary"});
            let error =
                parse_result(Some(2), b"", &serde_json::to_vec(&failure).unwrap()).unwrap_err();
            assert_eq!(
                serde_json::to_value(error).unwrap(),
                serde_json::json!({"code":expected})
            );
        }
    }

    #[test]
    fn cli_output_is_validated_and_error_messages_cannot_escape() {
        assert!(parse_result(
            Some(0),
            br#"{"schema":1,"status":"draft_ready","reconnected":false}"#,
            b""
        )
        .is_ok());
        for bytes in [
            b"private-canary".as_slice(),
            br#"{"schema":2,"status":"draft_ready","reconnected":false}"#,
        ] {
            assert_eq!(
                parse_result(Some(0), bytes, b"").unwrap_err().code,
                R2SetupErrorCode::SetupFailed
            );
        }
        let failure = br#"{"schema":1,"status":"error","exit_code":2,"code":"wrangler_not_found","message":"private-canary"}"#;
        let error = parse_result(Some(2), b"", failure).unwrap_err();
        assert_eq!(error.code, R2SetupErrorCode::WranglerNotFound);
        assert!(!serde_json::to_string(&error).unwrap().contains("canary"));
        assert_eq!(
            parse_result(Some(4), b"", failure).unwrap_err().code,
            R2SetupErrorCode::SetupFailed
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runner_bounds_output_and_stops_a_hung_process() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("fixture-cli");
        std::fs::write(&program, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(run_cli(
            &program,
            &[],
            Duration::from_millis(30),
            watch::channel(false).1,
            None
        )
        .await
        .is_err());
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf '%s' '{}'\n",
                "x".repeat(MAX_OUTPUT as usize + 1)
            ),
        )
        .unwrap();
        assert!(run_cli(
            &program,
            &[],
            Duration::from_secs(2),
            watch::channel(false).1,
            None
        )
        .await
        .is_err());
        std::fs::write(&program, "#!/bin/sh\nprintf '%s' '{\"schema\":1,\"status\":\"draft_ready\",\"reconnected\":false}'\n").unwrap();
        assert!(run_cli(
            &program,
            &[],
            Duration::from_secs(2),
            watch::channel(false).1,
            None
        )
        .await
        .is_ok());
    }

    #[test]
    fn cancellation_is_scoped_to_the_saved_revision_and_survives_early_delivery() {
        let control = R2SetupControl::default();
        let draft = StorageSetupDraftId::parse("storagedraft-111111111111111111111111").unwrap();
        control.cancel(draft.clone(), 1).unwrap();
        assert!(matches!(
            control.begin((draft.clone(), 1)),
            Err(R2SetupFailure {
                code: R2SetupErrorCode::Cancelled
            })
        ));
        let run = control.begin((draft.clone(), 2)).unwrap();
        control.cancel(draft.clone(), 1).unwrap();
        assert!(!*run.cancellation.borrow());
        assert!(matches!(
            control.begin((draft.clone(), 3)),
            Err(R2SetupFailure {
                code: R2SetupErrorCode::SetupBusy
            })
        ));
        control.cancel(draft.clone(), 2).unwrap();
        assert!(*run.cancellation.borrow());
        drop(run);
        // A delayed Cancel from the previous attempt cannot cancel the next revision.
        control.cancel(draft.clone(), 2).unwrap();
        let next = control.begin((draft, 3)).unwrap();
        assert!(!*next.cancellation.borrow());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancellation_stops_the_helper_and_its_child_before_retry() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("fixture-cli");
        std::fs::write(
            &program,
            "#!/bin/sh\n/bin/sleep 60 &\nprintf '%s' \"$!\" > \"$0.child\"\nwait\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let control = R2SetupControl::default();
        let draft = StorageSetupDraftId::parse("storagedraft-111111111111111111111111").unwrap();
        let run = control.begin((draft.clone(), 1)).unwrap();
        let child_path = program.with_extension("child");
        let cancellation = run.cancellation.clone();
        let command_program = program.clone();
        let runner = tokio::spawn(async move {
            run_cli(
                &command_program,
                &[],
                Duration::from_secs(5),
                cancellation,
                None,
            )
            .await
        });
        let child_pid = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(text) = std::fs::read_to_string(&child_path) {
                    if let Ok(pid) = text.parse::<i32>() {
                        break pid;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        control.cancel(draft.clone(), 1).unwrap();
        assert_eq!(
            runner.await.unwrap().unwrap_err().code,
            R2SetupErrorCode::Cancelled
        );
        tokio::time::timeout(Duration::from_secs(3), async {
            while unsafe { libc::kill(child_pid, 0) } == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the Wrangler child must exit before retry");
        drop(run);
        let next = control.begin((draft, 2)).unwrap();
        std::fs::write(&program,
            "#!/bin/sh\nprintf '%s' '{\"schema\":1,\"status\":\"draft_ready\",\"reconnected\":false}'\n").unwrap();
        assert!(run_cli(
            &program,
            &[],
            Duration::from_secs(2),
            next.cancellation.clone(),
            None
        )
        .await
        .is_ok());
    }

    #[tokio::test]
    async fn early_cancellation_does_not_launch_a_process() {
        let (_sender, receiver) = watch::channel(true);
        let error = run_cli(
            Path::new("/does-not-exist"),
            &[],
            Duration::from_secs(1),
            receiver,
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, R2SetupErrorCode::Cancelled);
    }
}
