//! The Cloudflare R2 storage adapter (`r2`, config version 1).
//!
//! R2 is reached through its S3 API. The version-1 settings are the canonical
//! S3 API URL (which names the account, jurisdiction, and bucket) and the
//! access key ID; the secret access key is the one secret. The R2
//! setup action runs the packaged Mallard CLI with local Wrangler and
//! prepares an ordinary storage draft for review.

use super::traits::{
    StorageAdapterError, StorageDiscoveryDescriptor, StorageDiscoveryHelp, StorageFieldDerivation,
    StorageFieldDescriptor, StorageFieldKind, StorageLocalSetup, StorageNameSuggestion,
    StorageProviderAdapter, StorageProviderDescriptor, StorageSecretDescriptor,
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
    r"^https://([0-9a-fA-F]{32})(?:\.(eu|us|fedramp))?\.r2\.cloudflarestorage\.com/([^/?#%]+)$";

/// Where a bucket's data may live. Cloudflare fixes it at creation; the S3
/// endpoint host and the management API's `cf-r2-jurisdiction` header both
/// name it by the same identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R2Jurisdiction {
    Default,
    Eu,
    Us,
    Fedramp,
}

impl R2Jurisdiction {
    /// Every jurisdiction Mallard can create in and scan, in the order the
    /// setup lane lists them.
    pub const ALL: [R2Jurisdiction; 4] = [Self::Default, Self::Eu, Self::Us, Self::Fedramp];

    /// The identifier Cloudflare uses in the API header and bucket listings.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Eu => "eu",
            Self::Us => "us",
            Self::Fedramp => "fedramp",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|jurisdiction| jurisdiction.as_str() == value)
    }

    /// The word the setup lane prints for it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Eu => "EU",
            Self::Us => "US",
            Self::Fedramp => "FedRAMP",
        }
    }

    /// The S3 endpoint for an account's buckets in this jurisdiction.
    pub fn endpoint(self, account_id: &str) -> String {
        match self {
            Self::Default => format!("https://{account_id}.r2.cloudflarestorage.com"),
            other => format!(
                "https://{account_id}.{}.r2.cloudflarestorage.com",
                other.as_str()
            ),
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
        [account, "us", "r2", "cloudflarestorage", "com"] => (*account, R2Jurisdiction::Us),
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
    if !valid_bucket_name(raw_bucket) {
        return Err(StorageAdapterError::invalid(
            "r2_bucket_name_invalid",
            "The R2 bucket name must use 3 to 63 lowercase letters, numbers, or hyphens and begin and end with a letter or number.",
        ));
    }

    let account_id = account_id.to_ascii_lowercase();
    let endpoint = jurisdiction.endpoint(&account_id);
    Ok(R2Destination {
        jurisdiction,
        account_id,
        bucket: raw_bucket.to_string(),
        api_url: format!("{endpoint}/{raw_bucket}"),
        endpoint,
    })
}

/// Cloudflare's bucket rule: 3 to 63 lowercase letters, numbers or hyphens,
/// beginning and ending with a letter or number.
pub fn valid_bucket_name(bucket: &str) -> bool {
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
            managed_secret_id: None,
            local_setup: Some(StorageLocalSetup::Wrangler),
            setup_guide_url: Some(R2_SETUP_DOC.to_string()),
            manual_setup_hint: Some("Enter an existing R2 connection in the fields below.".to_string()),
            discovery: Some(StorageDiscoveryDescriptor {
                unit: "bucket".to_string(),
                unit_plural: "buckets".to_string(),
                place: "your Cloudflare account".to_string(),
                create_hint: "A new bucket in your Cloudflare account, created with Wrangler or by hand.".to_string(),
                existing_hint: "Use a shared R2 bucket with your team, or connect your own.".to_string(),
                existing_help: Some(StorageDiscoveryHelp {
                    label: "Set up team access".to_string(),
                    url: "https://developers.cloudflare.com/fundamentals/manage-members/manage/".to_string(),
                }),
                find_label: "Find in my account".to_string(),
                access_hint: "Uses your Wrangler sign-in. Read-only: lists the buckets and reads one marker file in each.".to_string(),
                scanning_hint: "Looking through your Cloudflare account…".to_string(),
                prepare_hint: "Cloudflare issues an access key for this bucket only.".to_string(),
            }),
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
        for jurisdiction in R2Jurisdiction::ALL {
            let url = format!("{}/bucket-1", jurisdiction.endpoint(ACCOUNT));
            let parsed = parse_r2_api_url(&url).unwrap();
            assert_eq!(parsed.jurisdiction(), jurisdiction);
            assert_eq!(parsed.api_url(), url);
            assert_eq!(
                R2Jurisdiction::parse(jurisdiction.as_str()),
                Some(jurisdiction)
            );
        }
        assert_eq!(R2Jurisdiction::parse("fedramp-high"), None);
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
    fn r2_declares_local_wrangler_setup_without_an_agent() {
        let descriptor = R2Adapter.descriptor();
        assert_eq!(descriptor.local_setup, Some(StorageLocalSetup::Wrangler));
        assert!(descriptor.managed_secret_id.is_none());
    }
}
