//! The Cloudflare R2 storage adapter (`r2`, config version 1).
//!
//! R2 is reached through its S3 API. The version-1 settings are the canonical
//! S3 API URL (which names the account, jurisdiction, and bucket) and the
//! access key ID; the secret access key is the one secret. The R2
//! provisioning assistant — an agent runs the packaged Mallard CLI, which
//! drives Cloudflare's Wrangler tooling — is declared here and delivers its
//! result into an ordinary storage draft.

use super::traits::{
    StorageAdapterError, StorageFieldDerivation, StorageFieldDescriptor, StorageFieldKind,
    StorageNameSuggestion, StorageProviderAdapter, StorageProviderDescriptor,
    StorageSecretDescriptor, StorageSetupAssistantContext, StorageSetupAssistantDescriptor,
    StorageSummaryField,
};
use crate::storage_core::{ObjectStore, R2CapabilityProbeErrorKind, S3Connection, S3ObjectStore};
use mallard_domain::{
    BoundedConfigObject, FieldKey, SecretKey, StorageProviderConfig, StorageProviderKey,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::runtime::Handle;
use url::Url;

pub const R2_STORAGE_PROVIDER: &str = "r2";
pub const R2_CONFIG_VERSION: u32 = 1;
pub const R2_SECRET_ACCESS_KEY: &str = "secret_access_key";
const MAX_API_URL_BYTES: usize = 4 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 4 * 1024;

pub fn provider_key() -> StorageProviderKey {
    StorageProviderKey::parse(R2_STORAGE_PROVIDER).expect("static provider key")
}

pub fn secret_access_key() -> SecretKey {
    SecretKey::parse(R2_SECRET_ACCESS_KEY).expect("static secret key")
}

/// The version-1 settings shape. Unknown fields are rejected. The bucket and
/// account ID repeat what the S3 API URL names; the adapter requires them to
/// agree so a typo in either field is caught before any remote call.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct R2SettingsV1 {
    pub api_url: String,
    pub bucket: String,
    pub account_id: String,
    pub access_key_id: String,
}

/// The canonical settings for one parsed destination.
pub fn r2_settings_v1(destination: &R2Destination, access_key_id: &str) -> R2SettingsV1 {
    R2SettingsV1 {
        api_url: destination.api_url().to_string(),
        bucket: destination.bucket().to_string(),
        account_id: destination.account_id().to_string(),
        access_key_id: access_key_id.to_string(),
    }
}

const R2_SETUP_DOC: &str = "https://developers.cloudflare.com/r2/get-started/s3/";
const R2_BUCKET_DOC: &str = "https://developers.cloudflare.com/r2/buckets/create-buckets/";
const R2_CREDENTIALS_DOC: &str = "https://developers.cloudflare.com/r2/api/tokens/";
const CLOUDFLARE_ACCOUNT_ID_DOC: &str =
    "https://developers.cloudflare.com/fundamentals/account/find-account-and-zone-ids/";
/// The S3 API URL shape the form derives the bucket (group 3) and account ID
/// (group 1) from while the user types. `parse_r2_api_url` remains the
/// authority at validation time.
const R2_API_URL_DERIVATION_PATTERN: &str =
    r"^https://([0-9a-fA-F]{32})(?:\.(eu|fedramp))?\.r2\.cloudflarestorage\.com/([^/?#%]+)$";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R2Jurisdiction {
    Default,
    Eu,
    Fedramp,
}

impl R2Jurisdiction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Eu => "eu",
            Self::Fedramp => "fedramp",
        }
    }
}

/// Parsed canonical destination. `Serialize` is intentionally omitted and
/// `Debug` is redacted because completion and error output must not expose
/// destinations.
#[derive(Clone, PartialEq, Eq)]
pub struct R2Destination {
    jurisdiction: R2Jurisdiction,
    account_id: String,
    bucket: String,
    endpoint: String,
    api_url: String,
}

impl std::fmt::Debug for R2Destination {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("R2Destination(<redacted>)")
    }
}

impl R2Destination {
    pub fn jurisdiction(&self) -> R2Jurisdiction {
        self.jurisdiction
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn api_url(&self) -> &str {
        &self.api_url
    }
}

fn invalid_api_url() -> StorageAdapterError {
    StorageAdapterError::invalid("r2_api_url_invalid", "The R2 API URL is invalid.")
}

/// Parse one strict Cloudflare R2 S3 API URL with exactly one bucket path.
/// Public/custom domains and all URL features that can alias a destination
/// are rejected before canonicalization.
pub fn parse_r2_api_url(value: &str) -> Result<R2Destination, StorageAdapterError> {
    if value.is_empty()
        || value.len() > MAX_API_URL_BYTES
        || value.trim() != value
        || value.contains('\\')
    {
        return Err(invalid_api_url());
    }
    let Some((raw_scheme, after_scheme)) = value.split_once("://") else {
        return Err(invalid_api_url());
    };
    if !raw_scheme.eq_ignore_ascii_case("https") {
        return Err(StorageAdapterError::invalid(
            "r2_api_url_requires_https",
            "The R2 API URL must use HTTPS.",
        ));
    }
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let raw_authority = &after_scheme[..authority_end];
    let raw_path = &after_scheme[authority_end..];
    if raw_authority.is_empty() || raw_authority.contains('@') || raw_authority.contains(':') {
        return Err(StorageAdapterError::invalid(
            "r2_api_url_authority_not_allowed",
            "The R2 API URL must not contain credentials or a port.",
        ));
    }
    if value.contains(['?', '#']) {
        return Err(StorageAdapterError::invalid(
            "r2_api_url_suffix_not_allowed",
            "The R2 API URL must not contain a query string or fragment.",
        ));
    }
    let Some(raw_bucket) = raw_path.strip_prefix('/') else {
        return Err(StorageAdapterError::invalid(
            "r2_api_url_bucket_required",
            "The R2 API URL must end with exactly one bucket name.",
        ));
    };
    if raw_bucket.is_empty() || raw_bucket.contains(['/', '%']) {
        return Err(StorageAdapterError::invalid(
            "r2_api_url_bucket_required",
            "The R2 API URL must end with exactly one bucket name.",
        ));
    }

    let parsed = Url::parse(value).map_err(|_| invalid_api_url())?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid_api_url());
    }
    let host = parsed.host_str().ok_or_else(invalid_api_url)?;
    let labels = host.split('.').collect::<Vec<_>>();
    let (account_id, jurisdiction) = match labels.as_slice() {
        [account, "r2", "cloudflarestorage", "com"] => (*account, R2Jurisdiction::Default),
        [account, "eu", "r2", "cloudflarestorage", "com"] => (*account, R2Jurisdiction::Eu),
        [account, "fedramp", "r2", "cloudflarestorage", "com"] => {
            (*account, R2Jurisdiction::Fedramp)
        }
        _ => {
            return Err(StorageAdapterError::invalid(
                "r2_api_url_host_not_supported",
                "The R2 API URL must use a supported Cloudflare R2 S3 endpoint.",
            ))
        }
    };
    if account_id.len() != 32 || !account_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StorageAdapterError::invalid(
            "r2_account_id_invalid",
            "The R2 API URL account ID must be 32 hexadecimal characters.",
        ));
    }
    if !valid_bucket(raw_bucket) {
        return Err(StorageAdapterError::invalid(
            "r2_bucket_name_invalid",
            "The R2 bucket name must use 3 to 63 lowercase letters, numbers, or hyphens and begin and end with a letter or number.",
        ));
    }

    let account_id = account_id.to_ascii_lowercase();
    let endpoint = match jurisdiction {
        R2Jurisdiction::Default => {
            format!("https://{account_id}.r2.cloudflarestorage.com")
        }
        R2Jurisdiction::Eu => {
            format!("https://{account_id}.eu.r2.cloudflarestorage.com")
        }
        R2Jurisdiction::Fedramp => {
            format!("https://{account_id}.fedramp.r2.cloudflarestorage.com")
        }
    };
    Ok(R2Destination {
        jurisdiction,
        account_id,
        bucket: raw_bucket.to_string(),
        api_url: format!("{endpoint}/{raw_bucket}"),
        endpoint,
    })
}

fn valid_bucket(bucket: &str) -> bool {
    (3..=63).contains(&bucket.len())
        && bucket
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && bucket
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && bucket
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
}

fn validate_access_key_id(value: &str) -> Result<(), StorageAdapterError> {
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(StorageAdapterError::invalid(
            "r2_access_key_id_invalid",
            "The R2 Access Key ID is invalid.",
        ))
    } else {
        Ok(())
    }
}

/// The typed, validated form of one R2 configuration.
struct TypedR2Config {
    destination: R2Destination,
    access_key_id: String,
}

fn decode_settings(settings: &BoundedConfigObject) -> Result<TypedR2Config, StorageAdapterError> {
    let typed: R2SettingsV1 = settings.decode().map_err(|_| {
        StorageAdapterError::invalid(
            "r2_settings_invalid",
            "The R2 storage settings are incomplete or invalid.",
        )
    })?;
    let destination = parse_r2_api_url(&typed.api_url)?;
    if typed.bucket.trim().is_empty() {
        return Err(StorageAdapterError::invalid(
            "r2_bucket_required",
            "Enter the bucket name.",
        ));
    }
    if typed.bucket.trim() != destination.bucket() {
        return Err(StorageAdapterError::invalid(
            "r2_bucket_mismatch",
            "The bucket name must match the bucket in the S3 API URL.",
        ));
    }
    if typed.account_id.trim().is_empty() {
        return Err(StorageAdapterError::invalid(
            "r2_account_id_required",
            "Enter the Cloudflare Account ID.",
        ));
    }
    if !typed
        .account_id
        .trim()
        .eq_ignore_ascii_case(destination.account_id())
    {
        return Err(StorageAdapterError::invalid(
            "r2_account_id_mismatch",
            "The Account ID must match the account in the S3 API URL.",
        ));
    }
    validate_access_key_id(&typed.access_key_id)?;
    Ok(TypedR2Config {
        destination,
        access_key_id: typed.access_key_id,
    })
}

fn decode_config(config: &StorageProviderConfig) -> Result<TypedR2Config, StorageAdapterError> {
    if config.version != R2_CONFIG_VERSION {
        return Err(StorageAdapterError::invalid(
            "r2_config_version_unsupported",
            "The R2 storage configuration version is not supported.",
        ));
    }
    decode_settings(&config.settings)
}

fn require_secret(config: &StorageProviderConfig) -> Result<&str, StorageAdapterError> {
    let secret = config.secrets.get(&secret_access_key()).ok_or_else(|| {
        StorageAdapterError::invalid(
            "r2_secret_access_key_required",
            "The R2 Secret Access Key is required.",
        )
    })?;
    if secret.expose().len() > MAX_CREDENTIAL_BYTES {
        return Err(StorageAdapterError::invalid(
            "r2_secret_access_key_invalid",
            "The R2 Secret Access Key is invalid.",
        ));
    }
    Ok(secret.expose())
}

fn probe_error(error: crate::storage_core::R2CapabilityProbeError) -> StorageAdapterError {
    match error.kind() {
        R2CapabilityProbeErrorKind::Remote => StorageAdapterError::remote(
            "r2_capability_verification_failed",
            "R2 authentication or required remote capability verification failed.",
        ),
        R2CapabilityProbeErrorKind::CleanupRequired => StorageAdapterError::cleanup_required(
            "r2_probe_cleanup_required",
            "R2 remote cleanup is required",
        ),
    }
}

fn open_store(
    config: &StorageProviderConfig,
    runtime: Handle,
) -> Result<S3ObjectStore, StorageAdapterError> {
    let typed = decode_config(config)?;
    let secret = require_secret(config)?;
    S3ObjectStore::from_connection(
        &S3Connection {
            endpoint: typed.destination.endpoint(),
            region: "auto",
            bucket: typed.destination.bucket(),
            access_key_id: &typed.access_key_id,
            secret_access_key: secret,
        },
        runtime,
    )
    .map_err(|_| {
        StorageAdapterError::internal(
            "r2_client_unavailable",
            "Mallard could not create the R2 client.",
        )
    })
}

pub struct R2Adapter;

impl StorageProviderAdapter for R2Adapter {
    fn descriptor(&self) -> StorageProviderDescriptor {
        StorageProviderDescriptor {
            key: provider_key(),
            config_version: R2_CONFIG_VERSION,
            display_name: "Cloudflare R2".to_string(),
            description: "An R2 bucket reached through its S3 API.".to_string(),
            default_name: "R2 storage".to_string(),
            name_suggestion: Some(StorageNameSuggestion {
                source: FieldKey::parse("bucket").expect("static field key"),
                pattern: r"^\s*(\S.*?)\s*$".to_string(),
                group: 1,
            }),
            display_rank: 10,
            icon: "cloud".to_string(),
            clear_mode: super::traits::StorageClearMode::Unconditional,
            fields: vec![
                StorageFieldDescriptor {
                immutable_on_edit: false,
                    field_id: FieldKey::parse("api_url").expect("static field key"),
                    label: "S3 API URL".to_string(),
                    kind: StorageFieldKind::Url,
                    required: true,
                    placeholder: Some(
                        "Example: https://9cc0c910ec***41511aca1.r2.cloudflarestorage.com/agent"
                            .to_string(),
                    ),
                    help: Some("How to find the Cloudflare R2 S3 API URL".to_string()),
                    help_url: Some(R2_SETUP_DOC.to_string()),
                    derive: None,
                },
                StorageFieldDescriptor {
                immutable_on_edit: false,
                    field_id: FieldKey::parse("bucket").expect("static field key"),
                    label: "Bucket name".to_string(),
                    kind: StorageFieldKind::Text,
                    required: true,
                    placeholder: Some("Example: codex-sync-backups".to_string()),
                    help: Some("How to create a Cloudflare R2 bucket".to_string()),
                    help_url: Some(R2_BUCKET_DOC.to_string()),
                    derive: Some(StorageFieldDerivation {
                        source: FieldKey::parse("api_url").expect("static field key"),
                        pattern: R2_API_URL_DERIVATION_PATTERN.to_string(),
                        group: 3,
                    }),
                },
                StorageFieldDescriptor {
                immutable_on_edit: false,
                    field_id: FieldKey::parse("account_id").expect("static field key"),
                    label: "Account ID".to_string(),
                    kind: StorageFieldKind::Text,
                    required: true,
                    placeholder: Some("Example: 023e105f4ecef8ad9ca31a8372d0c353".to_string()),
                    help: Some("Where to find your Cloudflare Account ID".to_string()),
                    help_url: Some(CLOUDFLARE_ACCOUNT_ID_DOC.to_string()),
                    derive: Some(StorageFieldDerivation {
                        source: FieldKey::parse("api_url").expect("static field key"),
                        pattern: R2_API_URL_DERIVATION_PATTERN.to_string(),
                        group: 1,
                    }),
                },
                StorageFieldDescriptor {
                immutable_on_edit: false,
                    field_id: FieldKey::parse("access_key_id").expect("static field key"),
                    label: "Access Key ID".to_string(),
                    kind: StorageFieldKind::Text,
                    required: true,
                    placeholder: Some("Example: 4a2b…91ef".to_string()),
                    help: Some("How to create an R2 Access Key ID".to_string()),
                    help_url: Some(R2_CREDENTIALS_DOC.to_string()),
                    derive: None,
                },
            ],
            secrets: vec![StorageSecretDescriptor {
                secret_id: secret_access_key(),
                label: "Secret Access Key".to_string(),
                required: true,
                placeholder: Some("Example: a1B2…x9Y0".to_string()),
                help: Some("How to create an R2 Secret Access Key".to_string()),
                help_url: Some(R2_CREDENTIALS_DOC.to_string()),
            }],
            setup_assistant: Some(StorageSetupAssistantDescriptor {
                managed_secret_id: None,
                label: "Agent setup".to_string(),
                description: "Fills these values for your review — you complete Cloudflare sign-in, MFA, and consent in the browser, and finish setup yourself.".to_string(),
            }),
            setup_guide_url: Some(R2_SETUP_DOC.to_string()),
            manual_setup_hint: Some("Enter an existing R2 connection in the fields below.".to_string()),
        }
    }

    fn validate_settings(
        &self,
        settings: &BoundedConfigObject,
    ) -> Result<BoundedConfigObject, StorageAdapterError> {
        let typed = decode_settings(settings)?;
        BoundedConfigObject::encode(&r2_settings_v1(&typed.destination, &typed.access_key_id))
            .map_err(|_| invalid_api_url())
    }

    fn validate_config(&self, config: &StorageProviderConfig) -> Result<(), StorageAdapterError> {
        decode_config(config)?;
        require_secret(config)?;
        for key in config.secrets.keys() {
            if key != &secret_access_key() {
                return Err(StorageAdapterError::invalid(
                    "r2_secret_unknown",
                    "The R2 storage configuration carries an unknown secret.",
                ));
            }
        }
        Ok(())
    }

    fn summary(&self, config: &StorageProviderConfig) -> Result<Vec<StorageSummaryField>, String> {
        let typed = decode_config(config).map_err(|error| error.to_string())?;
        let field = |id: &str, label: &str, value: String| StorageSummaryField {
            field_id: FieldKey::parse(id).expect("static field key"),
            label: label.to_string(),
            value,
            is_path: false,
        };
        Ok(vec![
            field("bucket", "Bucket", typed.destination.bucket().to_string()),
            field(
                "account_id",
                "Account ID",
                typed.destination.account_id().to_string(),
            ),
            field(
                "endpoint",
                "S3 endpoint",
                typed.destination.endpoint().to_string(),
            ),
            field("access_key_id", "Access Key ID", typed.access_key_id),
        ])
    }

    fn continuity_coordinates(
        &self,
        config: &StorageProviderConfig,
    ) -> Result<Vec<String>, String> {
        let typed = decode_config(config).map_err(|error| error.to_string())?;
        Ok(vec![
            typed.destination.jurisdiction().as_str().to_string(),
            typed.destination.account_id().to_string(),
            typed.destination.bucket().to_string(),
        ])
    }

    fn local_root(&self, _config: &StorageProviderConfig) -> Result<Option<PathBuf>, String> {
        Ok(None)
    }

    fn journal_facts(&self, config: &StorageProviderConfig) -> Result<BoundedConfigObject, String> {
        let typed = decode_config(config).map_err(|error| error.to_string())?;
        BoundedConfigObject::from_json(&serde_json::json!({
            "account_id": typed.destination.account_id(),
            "bucket": typed.destination.bucket(),
            "jurisdiction": typed.destination.jurisdiction().as_str(),
        }))
    }

    fn probe(
        &self,
        config: &StorageProviderConfig,
        setup_id: &str,
        runtime: &Handle,
        _journal: &mut dyn super::traits::StorageProbeJournal,
    ) -> Result<(), StorageAdapterError> {
        let store = open_store(config, runtime.clone())?;
        store
            .probe_r2_capabilities(setup_id)
            .map(|_| ())
            .map_err(probe_error)
    }

    fn reconcile_probe(
        &self,
        config: &StorageProviderConfig,
        setup_id: &str,
        runtime: &Handle,
        _journal: &mut dyn super::traits::StorageProbeJournal,
    ) -> Result<(), StorageAdapterError> {
        let store = open_store(config, runtime.clone())?;
        store
            .reconcile_r2_capability_probe(setup_id)
            .map_err(probe_error)
    }

    fn setup_assistant_instruction(
        &self,
        context: &StorageSetupAssistantContext<'_>,
    ) -> Result<String, String> {
        Ok(assistant_instruction(context))
    }

    fn open(
        &self,
        config: &StorageProviderConfig,
        runtime: Option<Handle>,
    ) -> Result<Arc<dyn ObjectStore>, String> {
        let runtime =
            runtime.ok_or_else(|| "opening an R2 store requires a Tokio runtime".to_string())?;
        let store = open_store(config, runtime).map_err(|error| error.to_string())?;
        Ok(Arc::new(store))
    }
}

/// The instruction the agent receives: UI, browser consent, and credentials
/// stay outside the agent; it only runs the CLI against the exact draft.
fn assistant_instruction(context: &StorageSetupAssistantContext<'_>) -> String {
    let requested_name = {
        let trimmed = context.storage_name.trim();
        if trimmed.is_empty() {
            "Cloudflare R2"
        } else {
            trimmed
        }
    };
    let draft_id = context.draft_id;
    let draft_revision = context.draft_revision;
    [
        format!("Prepare the Cloudflare R2 storage “{requested_name}” draft through the Mallard CLI for my later review. Do not finish the setup for me. Do not invoke Computer Use for any purpose, including read-only verification."),
        String::new(),
        "Use this Mallard CLI:".to_string(),
        context.cli_path.to_string(),
        String::new(),
        "Treat that as a literal executable path and quote it for the current shell; do not interpret it as shell syntax.".to_string(),
        String::new(),
        "First check the required runtime dependency without installing anything: run `wrangler --version` and confirm that `wrangler` is available on PATH.".to_string(),
        String::new(),
        "If Wrangler is missing or `wrangler --version` fails, stop before invoking Mallard setup. Tell me that Mallard requires a working `wrangler` command on PATH, ask me to approve installing it, and show me this exact command: `npm install --global wrangler@latest`. Do not run that installation unless I explicitly approve it. Before an approved installation, run `npm --version`; if npm is missing, report that separately and ask how I want to install Node.js and npm—do not choose or install a package manager on your own. After an approved installation, rerun `wrangler --version` and continue only if it succeeds. Mallard invokes `wrangler` directly, so do not substitute `npx wrangler` or a project-local-only installation.".to_string(),
        String::new(),
        "Once the Wrangler dependency check succeeds, run the Mallard CLI’s `storage r2 --help` command.".to_string(),
        String::new(),
        "Before invoking setup, check whether your command-execution environment restricts writing to the current user’s ~/.mallard directory or launching the system browser. If it does, ask me for one-time approval to run the exact, fully quoted Mallard setup command described next with permission to write to the current user’s ~/.mallard directory and launch the system browser. Request approval only for that exact setup command, not for a shell or a broader command prefix. Do not invoke setup until I explicitly approve it; if I decline, stop.".to_string(),
        String::new(),
        format!("Then run its `storage r2 setup` command with all three required options: `--name` with the exact storage name shown above as one literal argument, `--draft-id` with the exact literal value `{draft_id}`, and `--expected-revision` with the exact decimal value `{draft_revision}`. Quote the executable path, storage name, and draft ID correctly for the current shell; do not interpret any value as shell syntax."),
        String::new(),
        "The Mallard CLI uses the existing Wrangler login and may open the system browser. I—not the agent—will review and complete Cloudflare login, MFA, and consent. Do not control the browser or inspect its contents. Wait for the CLI to finish; the agent should observe only its success or failure result.".to_string(),
        String::new(),
        "Complete only the draft-preparation handoff through the CLI. Do not work around an execution restriction by changing HOME, using temporary or project-local Mallard state, using sudo, editing ~/.mallard, or editing Mallard’s open setup form. Do not request or handle credentials, put a secret in an argument or environment variable, or print credentials.".to_string(),
        String::new(),
        "The setup command only prepares the exact schema-1 draft named above; it does not test the connection or commit the storage. Mallard is listening for that draft and will automatically fill the existing editable setup form when the CLI finishes. The CLI success or failure result is the end of the agent’s task. Do not use screenshots, browser automation, UI automation, or direct inspection of Mallard’s window or form to confirm the update. Report only the CLI success or failure result, then stop. I will confirm that the fields appeared, review or edit them, optionally test the connection, and personally click Finish setup when I am ready.".to_string(),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_domain::{SecretMap, SecretValue};

    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";

    /// Settings as the form submits them: bucket and account ID derived from
    /// the URL when it parses, empty otherwise.
    fn settings(api_url: &str, access_key_id: &str) -> BoundedConfigObject {
        let (bucket, account_id) = parse_r2_api_url(api_url)
            .map(|destination| {
                (
                    destination.bucket().to_string(),
                    destination.account_id().to_string(),
                )
            })
            .unwrap_or_default();
        BoundedConfigObject::from_json(&serde_json::json!({
            "api_url": api_url,
            "bucket": bucket,
            "account_id": account_id,
            "access_key_id": access_key_id,
        }))
        .unwrap()
    }

    fn config(secret: Option<&str>) -> StorageProviderConfig {
        let mut secrets = SecretMap::empty();
        if let Some(secret) = secret {
            secrets
                .insert(secret_access_key(), SecretValue::parse(secret).unwrap())
                .unwrap();
        }
        StorageProviderConfig::new(
            R2_CONFIG_VERSION,
            settings(
                &format!("https://{ACCOUNT}.eu.r2.cloudflarestorage.com/team-backup"),
                "access-canary",
            ),
            secrets,
        )
    }

    #[test]
    fn parser_canonicalizes_every_jurisdiction_and_rejects_aliases() {
        let upper = format!(
            "https://{}.R2.cloudflarestorage.com/bucket-1",
            ACCOUNT.to_ascii_uppercase()
        );
        let destination = parse_r2_api_url(&upper).unwrap();
        assert_eq!(destination.account_id(), ACCOUNT);
        assert_eq!(destination.jurisdiction(), R2Jurisdiction::Default);
        assert_eq!(
            destination.api_url(),
            format!("https://{ACCOUNT}.r2.cloudflarestorage.com/bucket-1")
        );
        for (url, code) in [
            (
                format!("http://{ACCOUNT}.r2.cloudflarestorage.com/b-1"),
                "r2_api_url_requires_https",
            ),
            (
                format!("https://user@{ACCOUNT}.r2.cloudflarestorage.com/b-1"),
                "r2_api_url_authority_not_allowed",
            ),
            (
                format!("https://{ACCOUNT}.r2.cloudflarestorage.com:443/b-1"),
                "r2_api_url_authority_not_allowed",
            ),
            (
                format!("https://{ACCOUNT}.r2.cloudflarestorage.com/b-1?x=1"),
                "r2_api_url_suffix_not_allowed",
            ),
            (
                format!("https://{ACCOUNT}.r2.cloudflarestorage.com"),
                "r2_api_url_bucket_required",
            ),
            (
                format!("https://{ACCOUNT}.r2.cloudflarestorage.com/a/b"),
                "r2_api_url_bucket_required",
            ),
            (
                format!("https://{ACCOUNT}.r2.cloudflarestorage.com/B"),
                "r2_bucket_name_invalid",
            ),
            (
                "https://example.com/bucket".to_string(),
                "r2_api_url_host_not_supported",
            ),
            (
                format!("https://{}.r2.cloudflarestorage.com/b-1", &ACCOUNT[..31]),
                "r2_account_id_invalid",
            ),
        ] {
            assert_eq!(parse_r2_api_url(&url).unwrap_err().code(), code, "{url}");
        }
    }

    #[test]
    fn settings_validation_canonicalizes_and_config_requires_the_secret() {
        let adapter = R2Adapter;
        let canonical = adapter
            .validate_settings(&settings(
                &format!(
                    "https://{}.eu.r2.cloudflarestorage.com/team-backup",
                    ACCOUNT.to_ascii_uppercase()
                ),
                "access-canary",
            ))
            .unwrap();
        assert_eq!(
            canonical.get_str("api_url").unwrap(),
            format!("https://{ACCOUNT}.eu.r2.cloudflarestorage.com/team-backup")
        );
        assert_eq!(
            adapter
                .validate_settings(&settings("https://example.com/bucket-1", "a"))
                .unwrap_err()
                .code(),
            "r2_api_url_host_not_supported"
        );
        assert_eq!(
            adapter
                .validate_settings(&settings(
                    &format!("https://{ACCOUNT}.r2.cloudflarestorage.com/bucket-1"),
                    " padded "
                ))
                .unwrap_err()
                .code(),
            "r2_access_key_id_invalid"
        );
        assert_eq!(
            adapter.validate_config(&config(None)).unwrap_err().code(),
            "r2_secret_access_key_required"
        );
        adapter
            .validate_config(&config(Some("secret-canary")))
            .unwrap();
        let extra = BoundedConfigObject::from_json(&serde_json::json!({
            "api_url": format!("https://{ACCOUNT}.r2.cloudflarestorage.com/b-1"),
            "bucket": "b-1",
            "account_id": ACCOUNT,
            "access_key_id": "a",
            "region": "auto",
        }))
        .unwrap();
        assert_eq!(
            adapter.validate_settings(&extra).unwrap_err().code(),
            "r2_settings_invalid"
        );
    }

    #[test]
    fn summaries_facts_and_coordinates_never_carry_the_secret() {
        let adapter = R2Adapter;
        let config = config(Some("secret-canary"));
        let summary = adapter.summary(&config).unwrap();
        let rendered = serde_json::to_string(&summary).unwrap();
        assert!(rendered.contains("team-backup"));
        assert!(rendered.contains("access-canary"));
        assert!(!rendered.contains("secret-canary"));
        assert_eq!(
            adapter.continuity_coordinates(&config).unwrap(),
            vec![
                "eu".to_string(),
                ACCOUNT.to_string(),
                "team-backup".to_string()
            ]
        );
        let facts = adapter.journal_facts(&config).unwrap();
        assert_eq!(facts.get_str("jurisdiction"), Some("eu"));
        assert_eq!(facts.get_str("bucket"), Some("team-backup"));
        assert!(!String::from_utf8(facts.canonical_json())
            .unwrap()
            .contains("canary"));
        assert_eq!(adapter.local_root(&config).unwrap(), None);
    }

    #[test]
    fn assistant_instruction_keeps_ui_browser_consent_and_credentials_outside_the_agent() {
        let adapter = R2Adapter;
        assert!(adapter.descriptor().setup_assistant.is_some());
        let instruction = adapter
            .setup_assistant_instruction(&StorageSetupAssistantContext {
                storage_name: "Team backup",
                cli_path: "/Applications/Mallard.app/Contents/MacOS/mallard-cli",
                draft_id: "draft-exact-123",
                draft_revision: 7,
                reconnect: false,
            })
            .unwrap();
        for expected in [
            "Team backup",
            "/Applications/Mallard.app/Contents/MacOS/mallard-cli",
            "storage r2 --help",
            "wrangler --version",
            "npm install --global wrangler@latest",
            "storage r2 setup",
            "--draft-id",
            "draft-exact-123",
            "exact decimal value `7`",
            "Do not invoke Computer Use",
            "Do not request or handle credentials",
            "personally click Finish setup",
        ] {
            assert!(instruction.contains(expected), "missing {expected:?}");
        }
        assert!(
            instruction.find("Do not invoke Computer Use").unwrap()
                < instruction.find("wrangler --version").unwrap()
        );
        assert!(
            instruction.find("storage r2 --help").unwrap()
                < instruction.find("one-time approval").unwrap()
        );
        assert!(
            instruction.find("one-time approval").unwrap()
                < instruction.find("storage r2 setup").unwrap()
        );
        assert!(!instruction.contains("secret-canary"));
    }
}
