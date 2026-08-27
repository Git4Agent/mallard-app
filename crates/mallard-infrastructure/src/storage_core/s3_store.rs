//! S3/R2 transport for the current bundle object-store contract.
//!
//! [`ObjectStore`] is deliberately synchronous because both storage
//! protocols also support a filesystem implementation.  Consequently, command
//! handlers must run every engine operation which can reach this store inside
//! `tokio::task::spawn_blocking`.  The store uses the supplied runtime handle
//! to drive AWS SDK futures; a nested-runtime panic is caught and converted to
//! an error with that instruction, but catching it is not a substitute for
//! keeping blocking work off an async runtime worker.

use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::{Client as S3Client, Config as S3Config};
use aws_smithy_http_client::{tls, Builder as HttpClientBuilder};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::future::Future;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::runtime::Handle;
use tokio::time::{sleep, timeout};

use super::{
    hex_lower, sha256 as sha256_bytes, CasExpectation, CasOutcome, ExpectedObject,
    ImmutablePutOutcome, ObjectKey, ObjectPrefix, ObjectStore, StageOutcome, StoreListPage,
    StoredMetadata, VerifiedBody,
};
use mallard_domain::{self as domain, StorageConfig, StorageKind};

const MAX_LIST_PAGE: usize = 10_000;
const S3_MAX_KEYS: usize = 1_000;
const MAX_LIST_REQUESTS: usize = MAX_LIST_PAGE + 2;
const R2_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const R2_SETUP_PROBE_PREFIX: &str = ".mallard/v1/_setup_probes";
const MAX_R2_SETUP_PROBE_BYTES: u64 = 512;
const R2_CREDENTIAL_READINESS_ATTEMPTS: usize = 10;
const R2_CREDENTIAL_READINESS_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R2CapabilityProbeErrorKind {
    Remote,
    CleanupRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum R2CapabilityProbeStage {
    Validate,
    List,
    Create,
    Read,
    ConditionalOverwrite,
    StaleConditionalRejection,
    Cleanup,
}

impl std::fmt::Display for R2CapabilityProbeStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Validate => "validation",
            Self::List => "bounded listing",
            Self::Create => "absent-key creation",
            Self::Read => "verified readback",
            Self::ConditionalOverwrite => "conditional overwrite",
            Self::StaleConditionalRejection => "stale-condition rejection",
            Self::Cleanup => "cleanup",
        })
    }
}

/// Sanitized capability-probe failure. It intentionally retains no SDK
/// error, endpoint, bucket, object key, or credential-bearing configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R2CapabilityProbeError {
    kind: R2CapabilityProbeErrorKind,
    stage: R2CapabilityProbeStage,
    setup_id: String,
}

impl R2CapabilityProbeError {
    fn remote(stage: R2CapabilityProbeStage, setup_id: &str) -> Self {
        Self {
            kind: R2CapabilityProbeErrorKind::Remote,
            stage,
            setup_id: setup_id.to_string(),
        }
    }

    fn cleanup_required(setup_id: &str) -> Self {
        Self {
            kind: R2CapabilityProbeErrorKind::CleanupRequired,
            stage: R2CapabilityProbeStage::Cleanup,
            setup_id: setup_id.to_string(),
        }
    }

    pub fn kind(&self) -> R2CapabilityProbeErrorKind {
        self.kind
    }

    pub fn stage(&self) -> R2CapabilityProbeStage {
        self.stage
    }

    pub fn setup_id(&self) -> &str {
        &self.setup_id
    }
}

impl std::fmt::Display for R2CapabilityProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            R2CapabilityProbeErrorKind::Remote => write!(
                formatter,
                "R2 capability verification failed during {}",
                self.stage
            ),
            R2CapabilityProbeErrorKind::CleanupRequired => write!(
                formatter,
                "R2 capability verification requires cleanup for setup '{}'",
                self.setup_id
            ),
        }
    }
}

impl std::error::Error for R2CapabilityProbeError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct R2CapabilityProbeReceipt {
    setup_id: String,
}

impl R2CapabilityProbeReceipt {
    pub fn setup_id(&self) -> &str {
        &self.setup_id
    }
}

#[derive(Clone)]
pub struct S3ObjectStore {
    client: S3Client,
    bucket: String,
    runtime: Handle,
    request_timeout: Duration,
}

impl S3ObjectStore {
    /// Build a store using the application's existing S3/R2 client
    /// configuration path.  An explicitly unsupported conditional-write
    /// capability is rejected: project bundles publish mutable heads only by CAS.
    pub fn from_config(config: &StorageConfig, runtime: Handle) -> Result<Self, String> {
        config.validate()?;
        if config.kind != StorageKind::S3 {
            return Err(format!(
                "storage '{}' is not an S3-compatible storage",
                config.id
            ));
        }
        if config.supports_conditional_writes == Some(false) {
            return Err(format!(
                "storage '{}' does not support the conditional writes required by project bundles",
                config.id
            ));
        }

        let client = make_s3_client(config)?;
        Self::from_client(client, config.bucket.clone(), runtime)
    }

    /// Convenience constructor for callers already running in a Tokio
    /// context.  Capture the handle before moving the engine operation into
    /// `spawn_blocking`.
    pub fn from_current_runtime(config: &StorageConfig) -> Result<Self, String> {
        let runtime = Handle::try_current()
            .map_err(|_| "creating an S3 bundle store requires a Tokio runtime".to_string())?;
        Self::from_config(config, runtime)
    }

    /// Construct from an already configured client.  This is useful for
    /// focused transport tests and for callers which own AWS client setup.
    pub fn from_client(
        client: S3Client,
        bucket: impl Into<String>,
        runtime: Handle,
    ) -> Result<Self, String> {
        let bucket = bucket.into();
        if bucket.is_empty()
            || bucket.len() > 1_024
            || bucket.trim() != bucket
            || bucket.chars().any(char::is_control)
        {
            return Err("S3 bundle store bucket is invalid".to_string());
        }
        Ok(Self {
            client,
            bucket,
            runtime,
            request_timeout: R2_REQUEST_TIMEOUT,
        })
    }

    fn run<F, T>(&self, operation: &str, future: F) -> Result<T, String>
    where
        F: Future<Output = Result<T, String>>,
    {
        match catch_unwind(AssertUnwindSafe(|| self.runtime.block_on(future))) {
            Ok(result) => result,
            Err(_) => Err(format!(
                "S3 bundle-store {} cannot block an async runtime worker; run the enclosing bundle-engine operation with tokio::task::spawn_blocking",
                operation
            )),
        }
    }

    /// Verify the exact R2 operations Mallard needs without widening the
    /// general [`ObjectStore`] namespace. Only a generated setup-probe key is
    /// accepted, and cleanup targets that one exact key.
    pub fn probe_r2_capabilities(
        &self,
        setup_id: &str,
    ) -> Result<R2CapabilityProbeReceipt, R2CapabilityProbeError> {
        let key = setup_probe_key(setup_id).map_err(|_| {
            R2CapabilityProbeError::remote(R2CapabilityProbeStage::Validate, setup_id)
        })?;
        match catch_unwind(AssertUnwindSafe(|| {
            self.runtime
                .block_on(self.probe_r2_capabilities_async(setup_id, &key))
        })) {
            Ok(result) => result,
            Err(_) => Err(R2CapabilityProbeError::remote(
                R2CapabilityProbeStage::Validate,
                setup_id,
            )),
        }
    }

    /// Remove and confirm absence of one previously journaled setup probe.
    /// This accepts the same narrow generated setup identifier as the probe
    /// itself and never exposes a general object-deletion primitive.
    pub fn reconcile_r2_capability_probe(
        &self,
        setup_id: &str,
    ) -> Result<(), R2CapabilityProbeError> {
        let key = setup_probe_key(setup_id).map_err(|_| {
            R2CapabilityProbeError::remote(R2CapabilityProbeStage::Validate, setup_id)
        })?;
        match catch_unwind(AssertUnwindSafe(|| {
            self.runtime.block_on(async {
                if self
                    .probe_key_is_absent_from_list_async(&key)
                    .await
                    .is_err()
                {
                    return false;
                }
                self.cleanup_probe_async(&key).await
            })
        })) {
            Ok(true) => Ok(()),
            Ok(false) | Err(_) => Err(R2CapabilityProbeError::cleanup_required(setup_id)),
        }
    }

    async fn probe_r2_capabilities_async(
        &self,
        setup_id: &str,
        key: &str,
    ) -> Result<R2CapabilityProbeReceipt, R2CapabilityProbeError> {
        match self.probe_key_is_absent_from_list_async(key).await {
            Ok(true) => {}
            Ok(false) | Err(()) => {
                return Err(R2CapabilityProbeError::remote(
                    R2CapabilityProbeStage::List,
                    setup_id,
                ))
            }
        }

        let first_body = format!("mallard-r2-setup-probe-v1:{setup_id}:first").into_bytes();
        match self
            .probe_conditional_put_async(key, &first_body, ProbeExpectation::Absent)
            .await
        {
            ProbePutAttempt::Written => {}
            ProbePutAttempt::Ambiguous => {
                return self
                    .probe_failure_after_possible_write(
                        setup_id,
                        key,
                        R2CapabilityProbeStage::Create,
                    )
                    .await
            }
            ProbePutAttempt::PreconditionFailed | ProbePutAttempt::Failed => {
                return Err(R2CapabilityProbeError::remote(
                    R2CapabilityProbeStage::Create,
                    setup_id,
                ))
            }
        }

        let first_etag = match self.probe_get_verified_async(key, &first_body).await {
            Ok(Some(etag)) => etag,
            Ok(None) | Err(()) => {
                return self
                    .probe_failure_after_possible_write(setup_id, key, R2CapabilityProbeStage::Read)
                    .await
            }
        };

        let second_body = format!("mallard-r2-setup-probe-v1:{setup_id}:second").into_bytes();
        match self
            .probe_conditional_put_async(key, &second_body, ProbeExpectation::Match(&first_etag))
            .await
        {
            ProbePutAttempt::Written => {}
            ProbePutAttempt::PreconditionFailed
            | ProbePutAttempt::Ambiguous
            | ProbePutAttempt::Failed => {
                return self
                    .probe_failure_after_possible_write(
                        setup_id,
                        key,
                        R2CapabilityProbeStage::ConditionalOverwrite,
                    )
                    .await
            }
        }

        let stale_body = format!("mallard-r2-setup-probe-v1:{setup_id}:stale").into_bytes();
        match self
            .probe_conditional_put_async(key, &stale_body, ProbeExpectation::Match(&first_etag))
            .await
        {
            ProbePutAttempt::PreconditionFailed => {}
            ProbePutAttempt::Written | ProbePutAttempt::Ambiguous | ProbePutAttempt::Failed => {
                return self
                    .probe_failure_after_possible_write(
                        setup_id,
                        key,
                        R2CapabilityProbeStage::StaleConditionalRejection,
                    )
                    .await
            }
        }

        if !self.cleanup_probe_async(key).await {
            return Err(R2CapabilityProbeError::cleanup_required(setup_id));
        }
        Ok(R2CapabilityProbeReceipt {
            setup_id: setup_id.to_string(),
        })
    }

    async fn probe_failure_after_possible_write(
        &self,
        setup_id: &str,
        key: &str,
        stage: R2CapabilityProbeStage,
    ) -> Result<R2CapabilityProbeReceipt, R2CapabilityProbeError> {
        if self.cleanup_probe_async(key).await {
            Err(R2CapabilityProbeError::remote(stage, setup_id))
        } else {
            Err(R2CapabilityProbeError::cleanup_required(setup_id))
        }
    }

    async fn probe_key_is_absent_from_list_async(&self, key: &str) -> Result<bool, ()> {
        for attempt in 0..R2_CREDENTIAL_READINESS_ATTEMPTS {
            let request = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(key)
                .max_keys(2)
                .send();
            let response = match timeout(self.request_timeout, request).await {
                Ok(Ok(response)) => response,
                Ok(Err(error))
                    if attempt + 1 < R2_CREDENTIAL_READINESS_ATTEMPTS
                        && matches!(sdk_status(&error), Some(401) | Some(403)) =>
                {
                    sleep(R2_CREDENTIAL_READINESS_DELAY).await;
                    continue;
                }
                Ok(Err(_)) | Err(_) => return Err(()),
            };
            if response.is_truncated() == Some(true) || response.contents().len() > 2 {
                return Err(());
            }
            return Ok(response.contents().is_empty());
        }
        Err(())
    }

    async fn probe_conditional_put_async(
        &self,
        key: &str,
        body: &[u8],
        expectation: ProbeExpectation<'_>,
    ) -> ProbePutAttempt {
        let checksum = aws_smithy_types::base64::encode(Sha256::digest(body));
        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .checksum_sha256(checksum)
            .body(ByteStream::from(Bytes::copy_from_slice(body)));
        request = match expectation {
            ProbeExpectation::Absent => request.if_none_match("*"),
            ProbeExpectation::Match(etag) => request.if_match(format!("\"{etag}\"")),
        };
        match timeout(self.request_timeout, request.send()).await {
            Err(_) => ProbePutAttempt::Ambiguous,
            Ok(Ok(_)) => ProbePutAttempt::Written,
            Ok(Err(error)) if matches!(sdk_status(&error), Some(409) | Some(412)) => {
                ProbePutAttempt::PreconditionFailed
            }
            Ok(Err(error)) if ambiguous_sdk_error(&error) => ProbePutAttempt::Ambiguous,
            Ok(Err(_)) => ProbePutAttempt::Failed,
        }
    }

    async fn probe_get_verified_async(
        &self,
        key: &str,
        expected: &[u8],
    ) -> Result<Option<String>, ()> {
        let request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send();
        let response = match timeout(self.request_timeout, request).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) if sdk_status(&error) == Some(404) => return Ok(None),
            Err(_) | Ok(Err(_)) => return Err(()),
        };
        if response
            .content_length()
            .is_some_and(|length| length < 0 || length as u64 > MAX_R2_SETUP_PROBE_BYTES)
        {
            return Err(());
        }
        let etag = response
            .e_tag()
            .and_then(|etag| validated_service_etag(etag, "setup probe").ok())
            .ok_or(())?;
        let mut body = response.body;
        let mut received = Vec::new();
        loop {
            let next = timeout(self.request_timeout, body.try_next())
                .await
                .map_err(|_| ())?
                .map_err(|_| ())?;
            let Some(chunk) = next else {
                break;
            };
            if received.len().saturating_add(chunk.len()) > MAX_R2_SETUP_PROBE_BYTES as usize {
                return Err(());
            }
            received.extend_from_slice(&chunk);
        }
        if received.len() != expected.len() || sha256_bytes(&received) != sha256_bytes(expected) {
            return Err(());
        }
        Ok(Some(etag))
    }

    async fn cleanup_probe_async(&self, key: &str) -> bool {
        let delete = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send();
        let _ = timeout(self.request_timeout, delete).await;

        let head = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send();
        match timeout(self.request_timeout, head).await {
            Ok(Err(error)) if sdk_status(&error) == Some(404) => true,
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => false,
        }
    }

    async fn get_response(
        &self,
        key: &ObjectKey,
    ) -> Result<Option<aws_sdk_s3::operation::get_object::GetObjectOutput>, String> {
        let request = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send();
        let response = match timeout(self.request_timeout, request).await {
            Err(_) => {
                return Err(format!(
                    "S3 get '{}' timed out after {} ms",
                    key,
                    self.request_timeout.as_millis()
                ));
            }
            Ok(Ok(response)) => response,
            Ok(Err(error)) if sdk_status(&error) == Some(404) => return Ok(None),
            Ok(Err(error)) => return Err(format!("S3 get '{}': {}", key, error)),
        };
        Ok(Some(response))
    }

    async fn get_metadata_async(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        let Some(response) = self.get_response(key).await? else {
            return Ok(None);
        };
        let declared_length = response.content_length();
        if let Some(length) = declared_length {
            if length < 0 {
                return Err(format!(
                    "S3 metadata object '{}' returned a negative Content-Length",
                    key
                ));
            }
            if length as u64 > max_bytes {
                return Err(format!("metadata object '{}' exceeds its read limit", key));
            }
        }
        let etag = response
            .e_tag()
            .ok_or_else(|| format!("S3 get '{}' returned no ETag", key))
            .and_then(|etag| validated_service_etag(etag, key.as_str()))?;
        let mut body = response.body;
        let mut bytes = Vec::new();
        if let Some(length) = declared_length {
            let capacity = usize::try_from(length)
                .map_err(|_| format!("metadata object '{}' is too large for this platform", key))?;
            bytes
                .try_reserve_exact(capacity)
                .map_err(|_| format!("reserve memory for metadata object '{}'", key))?;
        }
        let mut received = 0_u64;
        loop {
            let next = timeout(self.request_timeout, body.try_next())
                .await
                .map_err(|_| {
                    format!(
                        "S3 body '{}' timed out after {} ms of inactivity",
                        key,
                        self.request_timeout.as_millis()
                    )
                })?
                .map_err(|error| format!("S3 body '{}': {}", key, error))?;
            let Some(chunk) = next else {
                break;
            };
            let chunk_len = u64::try_from(chunk.len())
                .map_err(|_| format!("S3 metadata object '{}' byte count overflow", key))?;
            let new_received = received
                .checked_add(chunk_len)
                .ok_or_else(|| format!("S3 metadata object '{}' byte count overflow", key))?;
            if new_received > max_bytes {
                return Err(format!("metadata object '{}' exceeds its read limit", key));
            }
            bytes
                .try_reserve(chunk.len())
                .map_err(|_| format!("reserve memory for metadata object '{}'", key))?;
            bytes.extend_from_slice(&chunk);
            received = new_received;
        }
        Ok(Some(StoredMetadata { bytes, etag }))
    }

    async fn stage_verified_async(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<S3StageOutcome, String> {
        domain::validate_sha256("expected object", &expected.sha256)?;
        let Some(response) = self.get_response(key).await? else {
            return Ok(S3StageOutcome::Missing);
        };
        let declared_length = response.content_length();
        if let Some(length) = declared_length {
            if length < 0 {
                return Err(format!(
                    "S3 object '{}' returned a negative Content-Length",
                    key
                ));
            }
        }
        let etag = response
            .e_tag()
            .ok_or_else(|| format!("S3 get '{}' returned no ETag", key))
            .and_then(|etag| validated_service_etag(etag, key.as_str()))?;
        if declared_length.is_some_and(|length| length as u64 != expected.size) {
            return Ok(S3StageOutcome::Mismatch { etag });
        }

        let mut staged = NamedTempFile::new()
            .map_err(|error| format!("create staged S3 object '{}': {}", key, error))?;
        let mut body = response.body;
        let mut hasher = Sha256::new();
        let mut received = 0_u64;
        loop {
            let next = timeout(self.request_timeout, body.try_next())
                .await
                .map_err(|_| {
                    format!(
                        "S3 body '{}' timed out after {} ms of inactivity",
                        key,
                        self.request_timeout.as_millis()
                    )
                })?
                .map_err(|error| format!("S3 body '{}': {}", key, error))?;
            let Some(chunk) = next else {
                break;
            };
            let chunk_len = u64::try_from(chunk.len())
                .map_err(|_| format!("S3 object '{}' byte count overflow", key))?;
            let new_received = received
                .checked_add(chunk_len)
                .ok_or_else(|| format!("S3 object '{}' byte count overflow", key))?;
            if new_received > expected.size {
                return Ok(S3StageOutcome::Mismatch { etag });
            }
            staged
                .as_file_mut()
                .write_all(&chunk)
                .map_err(|error| format!("stage S3 object '{}': {}", key, error))?;
            hasher.update(&chunk);
            received = new_received;
        }
        let digest = hex_lower(&hasher.finalize());
        if received != expected.size || digest != expected.sha256 {
            return Ok(S3StageOutcome::Mismatch { etag });
        }
        staged
            .as_file_mut()
            .flush()
            .map_err(|error| format!("flush staged S3 object '{}': {}", key, error))?;
        staged
            .as_file_mut()
            .sync_all()
            .map_err(|error| format!("sync staged S3 object '{}': {}", key, error))?;
        Ok(S3StageOutcome::Verified(VerifiedBody::from_temp_file(
            staged, received, digest, etag,
        )?))
    }

    async fn conditional_put_async(
        &self,
        key: &ObjectKey,
        body: Bytes,
        expectation: &CasExpectation,
    ) -> Result<PutAttempt, String> {
        // The service verifies the received bytes against this digest before
        // accepting, so an acknowledged PUT is a content-verified receipt and
        // no write path ever reads its object back.
        let checksum = aws_smithy_types::base64::encode(Sha256::digest(&body));
        let mut request = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .checksum_sha256(checksum)
            .body(ByteStream::from(body));
        request = match expectation {
            CasExpectation::Absent => request.if_none_match("*"),
            CasExpectation::Match(etag) => {
                validate_expected_etag(etag)?;
                request.if_match(format!("\"{}\"", etag))
            }
        };

        match timeout(self.request_timeout, request.send()).await {
            Err(_) => Ok(PutAttempt::Ambiguous(format!(
                "request timed out after {} ms",
                self.request_timeout.as_millis()
            ))),
            Ok(Ok(response)) => {
                let etag = response
                    .e_tag()
                    .map(|etag| validated_service_etag(etag, key.as_str()))
                    .transpose()?;
                Ok(PutAttempt::Written(etag))
            }
            Ok(Err(error)) if matches!(sdk_status(&error), Some(409) | Some(412)) => {
                Ok(PutAttempt::PreconditionFailed)
            }
            Ok(Err(error)) if ambiguous_sdk_error(&error) => {
                Ok(PutAttempt::Ambiguous(error.to_string()))
            }
            Ok(Err(error)) => Err(format!("S3 conditional put '{}': {}", key, error)),
        }
    }

    async fn put_immutable_async(
        &self,
        key: &ObjectKey,
        bytes: Bytes,
    ) -> Result<ImmutablePutOutcome, String> {
        let expected = expected_object(&bytes)?;
        match self
            .conditional_put_async(key, bytes.clone(), &CasExpectation::Absent)
            .await?
        {
            PutAttempt::Written(_) => Ok(ImmutablePutOutcome::Written),
            PutAttempt::PreconditionFailed => {
                match self.stage_verified_async(key, &expected).await? {
                    S3StageOutcome::Verified(_) => Ok(ImmutablePutOutcome::AlreadyPresent),
                    S3StageOutcome::Mismatch { .. } => Err(format!(
                        "immutable object '{}' already exists with different bytes",
                        key
                    )),
                    S3StageOutcome::Missing => Err(format!(
                    "immutable put '{}' failed its absent precondition but the object is missing",
                    key
                )),
                }
            }
            PutAttempt::Ambiguous(cause) => match self.stage_verified_async(key, &expected).await {
                Ok(S3StageOutcome::Verified(_)) => Ok(ImmutablePutOutcome::AlreadyPresent),
                Ok(S3StageOutcome::Mismatch { .. }) => Err(format!(
                    "immutable put '{}' was ambiguous ({}) and a different object is present",
                    key, cause
                )),
                Ok(S3StageOutcome::Missing) => Err(format!(
                    "immutable put '{}' was ambiguous ({}) and no object is present",
                    key, cause
                )),
                Err(read_error) => Err(format!(
                    "immutable put '{}' was ambiguous ({}); resolution read failed: {}",
                    key, cause, read_error
                )),
            },
        }
    }

    async fn compare_and_swap_async(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: Bytes,
    ) -> Result<CasOutcome, String> {
        match self
            .conditional_put_async(key, bytes.clone(), expectation)
            .await?
        {
            PutAttempt::Written(Some(etag)) => Ok(CasOutcome::Written { etag }),
            PutAttempt::Written(None) | PutAttempt::Ambiguous(_) => {
                self.resolve_cas_write(key, &bytes).await
            }
            PutAttempt::PreconditionFailed => {
                let current = self
                    .stage_verified_async(key, &expected_object(&bytes)?)
                    .await?;
                Ok(CasOutcome::Conflict {
                    current_etag: current.etag().map(str::to_string),
                })
            }
        }
    }

    async fn resolve_cas_write(
        &self,
        key: &ObjectKey,
        intended_bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        match self
            .stage_verified_async(key, &expected_object(intended_bytes)?)
            .await?
        {
            S3StageOutcome::Verified(body) => Ok(CasOutcome::Written {
                etag: body.etag().to_string(),
            }),
            S3StageOutcome::Mismatch { etag } => Ok(CasOutcome::Conflict {
                current_etag: Some(etag),
            }),
            S3StageOutcome::Missing => Ok(CasOutcome::Conflict { current_etag: None }),
        }
    }

    async fn delete_async(&self, key: &ObjectKey) -> Result<(), String> {
        let request = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send();
        match timeout(self.request_timeout, request).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) if sdk_status(&error) == Some(404) => Ok(()),
            Ok(Err(error)) if ambiguous_sdk_error(&error) => {
                self.resolve_ambiguous_delete(key, &error.to_string()).await
            }
            Ok(Err(error)) => Err(format!("S3 delete '{}': {}", key, error)),
            Err(_) => {
                self.resolve_ambiguous_delete(
                    key,
                    &format!(
                        "request timed out after {} ms",
                        self.request_timeout.as_millis()
                    ),
                )
                .await
            }
        }
    }

    async fn resolve_ambiguous_delete(&self, key: &ObjectKey, cause: &str) -> Result<(), String> {
        let request = self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key.as_str())
            .send();
        match timeout(self.request_timeout, request).await {
            Ok(Err(error)) if sdk_status(&error) == Some(404) => Ok(()),
            Ok(Ok(_)) => Err(format!(
                "S3 delete '{}' was ambiguous ({}) and the object is still present",
                key, cause
            )),
            Ok(Err(error)) => Err(format!(
                "S3 delete '{}' was ambiguous ({}); resolution read failed: {}",
                key, cause, error
            )),
            Err(_) => Err(format!(
                "S3 delete '{}' was ambiguous ({}); resolution read timed out",
                key, cause
            )),
        }
    }

    async fn list_async(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        let target = limit + 1;
        let mut keys = BTreeSet::new();
        let mut continuation: Option<String> = None;
        let mut start_after = cursor.map(str::to_string);
        let mut request_count = 0_usize;

        loop {
            request_count += 1;
            if request_count > MAX_LIST_REQUESTS {
                return Err(format!(
                    "S3 list '{}' exceeded its pagination safety limit",
                    prefix.as_str()
                ));
            }
            let remaining = target.saturating_sub(keys.len()).max(1);
            let mut request = self
                .client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(prefix.as_str())
                .max_keys(remaining.min(S3_MAX_KEYS) as i32);
            if let Some(token) = &continuation {
                request = request.continuation_token(token);
            } else if let Some(after) = &start_after {
                request = request.start_after(after);
            }
            let response = timeout(self.request_timeout, request.send())
                .await
                .map_err(|_| {
                    format!(
                        "S3 list '{}' timed out after {} ms",
                        prefix.as_str(),
                        self.request_timeout.as_millis()
                    )
                })?
                .map_err(|error| format!("S3 list '{}': {}", prefix.as_str(), error))?;

            let mut last_raw_key: Option<String> = None;
            for object in response.contents() {
                let raw_key = object.key().ok_or_else(|| {
                    format!(
                        "S3 list '{}' returned an object without a key",
                        prefix.as_str()
                    )
                })?;
                if last_raw_key.as_deref().is_none_or(|last| raw_key > last) {
                    last_raw_key = Some(raw_key.to_string());
                }
                // S3's byte-prefix match can also return lexical neighbors
                // (for example `.mallard/v1/repositories-old` for the
                // repository namespace). Ignore
                // those before applying the stricter ObjectKey validator;
                // malformed keys inside the requested namespace still fail.
                if !raw_key_is_under_prefix(raw_key, prefix) {
                    continue;
                }
                let key = ObjectKey::parse(raw_key.to_string())?;
                if cursor.is_none_or(|cursor| key.as_str() > cursor) {
                    keys.insert(key);
                }
            }

            if keys.len() >= target {
                break;
            }
            if response.is_truncated() != Some(true) {
                break;
            }
            if let Some(next) = response.next_continuation_token() {
                if continuation.as_deref() == Some(next) {
                    return Err(format!(
                        "S3 list '{}' repeated a continuation token",
                        prefix.as_str()
                    ));
                }
                continuation = Some(next.to_string());
                start_after = None;
            } else if let Some(last) = last_raw_key {
                if start_after.as_deref() == Some(last.as_str()) {
                    return Err(format!(
                        "S3 list '{}' made no pagination progress",
                        prefix.as_str()
                    ));
                }
                continuation = None;
                start_after = Some(last);
            } else {
                return Err(format!(
                    "S3 list '{}' was truncated without a continuation token or key",
                    prefix.as_str()
                ));
            }
        }

        let has_more = keys.len() > limit;
        let mut keys = keys.into_iter().take(limit).collect::<Vec<_>>();
        keys.sort();
        let next_cursor = has_more.then(|| {
            keys.last()
                .expect("a nonzero list limit with more results has a last key")
                .as_str()
                .to_string()
        });
        Ok(StoreListPage { keys, next_cursor })
    }
}

impl ObjectStore for S3ObjectStore {
    fn get_metadata(
        &self,
        key: &ObjectKey,
        max_bytes: u64,
    ) -> Result<Option<StoredMetadata>, String> {
        self.run("get metadata", self.get_metadata_async(key, max_bytes))
    }

    fn stage_verified(
        &self,
        key: &ObjectKey,
        expected: &ExpectedObject,
    ) -> Result<StageOutcome, String> {
        self.run("stage verified", async {
            Ok(self
                .stage_verified_async(key, expected)
                .await?
                .into_public())
        })
    }

    /// Bounded-concurrency verified staging. Order matches `requests`; the
    /// first failure aborts and drains every remaining transfer.
    fn stage_many_verified(
        &self,
        requests: &[(ObjectKey, ExpectedObject)],
    ) -> Result<Vec<StageOutcome>, String> {
        if requests.len() <= 1 {
            return requests
                .iter()
                .map(|(key, expected)| self.stage_verified(key, expected))
                .collect();
        }
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
            crate::app_settings::cloud_transfer_concurrency(),
        ));
        self.run("bulk verified staging", async {
            let mut join_set = tokio::task::JoinSet::new();
            for (index, (key, expected)) in requests.iter().cloned().enumerate() {
                let store = self.clone();
                let semaphore = semaphore.clone();
                join_set.spawn(async move {
                    let _permit = semaphore
                        .acquire_owned()
                        .await
                        .map_err(|_| "bulk staging concurrency gate closed".to_string())?;
                    Ok::<_, String>((
                        index,
                        store
                            .stage_verified_async(&key, &expected)
                            .await?
                            .into_public(),
                    ))
                });
            }
            let mut outcomes: Vec<Option<StageOutcome>> = Vec::new();
            outcomes.resize_with(requests.len(), || None);
            while let Some(joined) = join_set.join_next().await {
                let worker_result = match joined {
                    Ok(result) => result,
                    Err(error) => {
                        abort_and_drain(&mut join_set).await;
                        return Err(format!("bulk staging worker failed: {}", error));
                    }
                };
                let (index, outcome) = match worker_result {
                    Ok(result) => result,
                    Err(error) => {
                        abort_and_drain(&mut join_set).await;
                        return Err(error);
                    }
                };
                outcomes[index] = Some(outcome);
            }
            Ok(outcomes
                .into_iter()
                .map(|slot| slot.expect("every bulk-staging index resolves"))
                .collect())
        })
    }

    fn put_immutable(&self, key: &ObjectKey, bytes: &[u8]) -> Result<ImmutablePutOutcome, String> {
        self.run(
            "immutable put",
            self.put_immutable_async(key, Bytes::copy_from_slice(bytes)),
        )
    }

    /// Bounded-concurrency bulk upload. Outcomes line up with `objects`; the
    /// first failure aborts the remaining transfers, like the sequential loop
    /// it replaces.
    fn put_many(
        &self,
        objects: Vec<(ObjectKey, Bytes)>,
    ) -> Result<Vec<ImmutablePutOutcome>, String> {
        if objects.len() <= 1 {
            return objects
                .into_iter()
                .map(|(key, bytes)| {
                    self.run("immutable put", self.put_immutable_async(&key, bytes))
                })
                .collect();
        }
        let count = objects.len();
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(
            crate::app_settings::cloud_transfer_concurrency(),
        ));
        self.run("bulk put", async {
            let mut join_set = tokio::task::JoinSet::new();
            for (index, (key, bytes)) in objects.into_iter().enumerate() {
                let store = self.clone();
                let semaphore = semaphore.clone();
                join_set.spawn(async move {
                    let _permit = semaphore
                        .acquire_owned()
                        .await
                        .map_err(|_| "bulk put concurrency gate closed".to_string())?;
                    Ok::<_, String>((index, store.put_immutable_async(&key, bytes).await?))
                });
            }
            let mut outcomes: Vec<Option<ImmutablePutOutcome>> = Vec::new();
            outcomes.resize_with(count, || None);
            while let Some(joined) = join_set.join_next().await {
                let (index, outcome) =
                    joined.map_err(|error| format!("bulk put worker failed: {}", error))??;
                outcomes[index] = Some(outcome);
            }
            Ok(outcomes
                .into_iter()
                .map(|slot| slot.expect("every bulk-put index resolves"))
                .collect())
        })
    }

    fn compare_and_swap(
        &self,
        key: &ObjectKey,
        expectation: &CasExpectation,
        bytes: &[u8],
    ) -> Result<CasOutcome, String> {
        if let CasExpectation::Match(etag) = expectation {
            validate_expected_etag(etag)?;
        }
        self.run(
            "compare-and-swap",
            self.compare_and_swap_async(key, expectation, Bytes::copy_from_slice(bytes)),
        )
    }

    fn delete(&self, key: &ObjectKey) -> Result<(), String> {
        self.run("delete", self.delete_async(key))
    }

    fn list(
        &self,
        prefix: &ObjectPrefix,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<StoreListPage, String> {
        if limit == 0 || limit > MAX_LIST_PAGE {
            return Err(format!(
                "list limit must be between 1 and {}",
                MAX_LIST_PAGE
            ));
        }
        if let Some(cursor) = cursor {
            let cursor_key = ObjectKey::parse(cursor.to_string())?;
            if !key_is_under_prefix(&cursor_key, prefix) {
                return Err(format!(
                    "list cursor '{}' is outside prefix '{}'",
                    cursor,
                    prefix.as_str()
                ));
            }
        }
        self.run("list", self.list_async(prefix, cursor, limit))
    }
}

enum ProbeExpectation<'a> {
    Absent,
    Match(&'a str),
}

enum ProbePutAttempt {
    Written,
    PreconditionFailed,
    Ambiguous,
    Failed,
}

fn setup_probe_key(setup_id: &str) -> Result<String, ()> {
    let Some(suffix) = setup_id.strip_prefix("r2setup-") else {
        return Err(());
    };
    if suffix.len() != 24
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(());
    }
    Ok(format!("{R2_SETUP_PROBE_PREFIX}/{setup_id}"))
}

enum PutAttempt {
    Written(Option<String>),
    PreconditionFailed,
    Ambiguous(String),
}

enum S3StageOutcome {
    Missing,
    Mismatch { etag: String },
    Verified(VerifiedBody),
}

impl S3StageOutcome {
    fn etag(&self) -> Option<&str> {
        match self {
            Self::Missing => None,
            Self::Mismatch { etag } => Some(etag),
            Self::Verified(body) => Some(body.etag()),
        }
    }

    fn into_public(self) -> StageOutcome {
        match self {
            Self::Missing => StageOutcome::Missing,
            Self::Mismatch { .. } => StageOutcome::Mismatch,
            Self::Verified(body) => StageOutcome::Verified(body),
        }
    }
}

async fn abort_and_drain<T: 'static>(join_set: &mut tokio::task::JoinSet<T>) {
    join_set.abort_all();
    while join_set.join_next().await.is_some() {}
}

fn expected_object(bytes: &[u8]) -> Result<ExpectedObject, String> {
    Ok(ExpectedObject {
        size: u64::try_from(bytes.len())
            .map_err(|_| "S3 object size cannot fit in u64".to_string())?,
        sha256: sha256_bytes(bytes),
    })
}

fn key_is_under_prefix(key: &ObjectKey, prefix: &ObjectPrefix) -> bool {
    raw_key_is_under_prefix(key.as_str(), prefix)
}

fn raw_key_is_under_prefix(key: &str, prefix: &ObjectPrefix) -> bool {
    key == prefix.as_str()
        || key
            .strip_prefix(prefix.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn make_s3_client(config: &StorageConfig) -> Result<S3Client, String> {
    if config.bucket.is_empty() {
        return Err("Bucket is not configured".to_string());
    }
    if config.access_key_id.is_empty() {
        return Err("R2 Access Key ID is not configured".to_string());
    }
    if config.secret_access_key.is_empty() {
        return Err("R2 Secret Access Key is not configured".to_string());
    }
    let endpoint = if !config.s3_endpoint.is_empty() {
        config.s3_endpoint.clone()
    } else if !config.account_id.is_empty() {
        format!("https://{}.r2.cloudflarestorage.com", config.account_id)
    } else {
        return Err("Endpoint is not configured".to_string());
    };
    let credentials = Credentials::new(
        &config.access_key_id,
        &config.secret_access_key,
        None,
        None,
        "mallard-project-sync",
    );
    let region = if config.region.is_empty() {
        "auto".to_string()
    } else {
        config.region.clone()
    };
    let sdk_config = S3Config::builder()
        .credentials_provider(credentials)
        .region(Region::new(region))
        .endpoint_url(endpoint)
        .force_path_style(true)
        .behavior_version_latest()
        .http_client(
            HttpClientBuilder::new()
                .tls_provider(tls::Provider::Rustls(
                    tls::rustls_provider::CryptoMode::AwsLc,
                ))
                .build_https(),
        )
        .build();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        S3Client::from_conf(sdk_config)
    }))
    .map_err(|panic| {
        if let Some(message) = panic.downcast_ref::<&str>() {
            format!("R2 client creation panicked: {message}")
        } else if let Some(message) = panic.downcast_ref::<String>() {
            format!("R2 client creation panicked: {message}")
        } else {
            "R2 client creation panicked".to_string()
        }
    })
}

fn sdk_status<E>(error: &SdkError<E>) -> Option<u16> {
    match error {
        SdkError::ServiceError(context) => Some(context.raw().status().as_u16()),
        SdkError::ResponseError(context) => Some(context.raw().status().as_u16()),
        _ => None,
    }
}

fn normalize_etag(etag: &str) -> String {
    etag.trim_matches('"').to_string()
}

fn validated_service_etag(raw: &str, key: &str) -> Result<String, String> {
    let etag = normalize_etag(raw);
    validate_expected_etag(&etag)
        .map_err(|_| format!("S3 object '{}' returned an invalid ETag", key))?;
    Ok(etag)
}

fn validate_expected_etag(etag: &str) -> Result<(), String> {
    if etag.is_empty()
        || etag.len() > 1_024
        || !etag.is_ascii()
        || etag
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'"' | b'\\'))
    {
        return Err("S3 CAS expectation contains an invalid ETag".to_string());
    }
    Ok(())
}

fn ambiguous_sdk_error<E>(error: &SdkError<E>) -> bool {
    matches!(
        error,
        SdkError::TimeoutError(_) | SdkError::DispatchFailure(_) | SdkError::ResponseError(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle_engine::BundleEngine;
    use aws_sdk_s3::config::{Credentials, Region};
    use aws_sdk_s3::Config as S3Config;
    use aws_smithy_http_client::{tls, Builder as HttpClientBuilder};
    use http_body_util::BodyExt;
    use hyper::body::{Body, Bytes as HyperBytes, Frame, Incoming, SizeHint};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use mallard_domain::StorageId;
    use std::collections::{BTreeMap, VecDeque};
    use std::convert::Infallible;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    const BUCKET: &str = "bundle-tests";

    struct TestS3 {
        endpoint: String,
        objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        get_behaviors: Arc<Mutex<BTreeMap<String, GetBehavior>>>,
        honor_conditions: Arc<AtomicBool>,
        fail_delete: Arc<AtomicBool>,
        list_authorization_failures: Arc<AtomicUsize>,
        server: tokio::task::JoinHandle<()>,
    }

    #[derive(Clone)]
    struct GetBehavior {
        chunks: Vec<Vec<u8>>,
        declared_length: Option<u64>,
        stall_after_chunks: bool,
    }

    struct TestBody {
        chunks: VecDeque<HyperBytes>,
        stall_after_chunks: bool,
    }

    impl TestBody {
        fn full(bytes: impl Into<HyperBytes>) -> Self {
            Self {
                chunks: VecDeque::from([bytes.into()]),
                stall_after_chunks: false,
            }
        }

        fn streaming(chunks: Vec<Vec<u8>>, stall_after_chunks: bool) -> Self {
            Self {
                chunks: chunks.into_iter().map(HyperBytes::from).collect(),
                stall_after_chunks,
            }
        }
    }

    impl Body for TestBody {
        type Data = HyperBytes;
        type Error = Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            if let Some(chunk) = self.chunks.pop_front() {
                return Poll::Ready(Some(Ok(Frame::data(chunk))));
            }
            if self.stall_after_chunks {
                Poll::Pending
            } else {
                Poll::Ready(None)
            }
        }

        fn is_end_stream(&self) -> bool {
            self.chunks.is_empty() && !self.stall_after_chunks
        }

        fn size_hint(&self) -> SizeHint {
            SizeHint::default()
        }
    }

    impl TestS3 {
        async fn start() -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind test S3");
            let endpoint = format!("http://{}", listener.local_addr().expect("test S3 address"));
            let objects = Arc::new(Mutex::new(BTreeMap::new()));
            let get_behaviors = Arc::new(Mutex::new(BTreeMap::new()));
            let honor_conditions = Arc::new(AtomicBool::new(true));
            let fail_delete = Arc::new(AtomicBool::new(false));
            let list_authorization_failures = Arc::new(AtomicUsize::new(0));
            let server_objects = objects.clone();
            let server_get_behaviors = get_behaviors.clone();
            let server_honor_conditions = honor_conditions.clone();
            let server_fail_delete = fail_delete.clone();
            let server_list_authorization_failures = list_authorization_failures.clone();
            let server = tokio::spawn(async move {
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        break;
                    };
                    let objects = server_objects.clone();
                    let get_behaviors = server_get_behaviors.clone();
                    let honor_conditions = server_honor_conditions.clone();
                    let fail_delete = server_fail_delete.clone();
                    let list_authorization_failures = server_list_authorization_failures.clone();
                    tokio::spawn(async move {
                        let service = service_fn(move |request| {
                            test_s3_request(
                                request,
                                objects.clone(),
                                get_behaviors.clone(),
                                honor_conditions.clone(),
                                fail_delete.clone(),
                                list_authorization_failures.clone(),
                            )
                        });
                        let _ = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    });
                }
            });
            Self {
                endpoint,
                objects,
                get_behaviors,
                honor_conditions,
                fail_delete,
                list_authorization_failures,
                server,
            }
        }

        fn seed(&self, key: String, bytes: Vec<u8>) {
            self.objects
                .lock()
                .expect("test S3 lock")
                .insert(key, bytes);
        }

        fn seed_streaming(
            &self,
            key: String,
            chunks: Vec<Vec<u8>>,
            declared_length: Option<u64>,
            stall_after_chunks: bool,
        ) {
            self.get_behaviors.lock().expect("test S3 lock").insert(
                key,
                GetBehavior {
                    chunks,
                    declared_length,
                    stall_after_chunks,
                },
            );
        }

        fn store(&self) -> S3ObjectStore {
            let credentials = Credentials::new("test", "test", None, None, "bundle-tests");
            let config = S3Config::builder()
                .credentials_provider(credentials)
                .region(Region::new("us-east-1"))
                .endpoint_url(&self.endpoint)
                .force_path_style(true)
                .behavior_version_latest()
                .http_client(
                    HttpClientBuilder::new()
                        .tls_provider(tls::Provider::Rustls(
                            tls::rustls_provider::CryptoMode::AwsLc,
                        ))
                        .build_https(),
                )
                .build();
            S3ObjectStore::from_client(S3Client::from_conf(config), BUCKET, Handle::current())
                .expect("construct test store")
        }

        fn ignore_conditions(&self) {
            self.honor_conditions.store(false, Ordering::SeqCst);
        }

        fn fail_delete(&self) {
            self.fail_delete.store(true, Ordering::SeqCst);
        }

        fn fail_list_authorization(&self, attempts: usize) {
            self.list_authorization_failures
                .store(attempts, Ordering::SeqCst);
        }
    }

    impl Drop for TestS3 {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    async fn blocking<T: Send + 'static>(operation: impl FnOnce() -> T + Send + 'static) -> T {
        tokio::task::spawn_blocking(operation)
            .await
            .expect("blocking store operation")
    }

    fn test_upload_key(name: &str) -> ObjectKey {
        ObjectKey::parse(format!(
            ".mallard/v1/repositories/0123456789abcdef0123456789abcdef/_uploads/tests/{name}"
        ))
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn engine_initializes_the_mallard_storage_marker_once() {
        let server = TestS3::start().await;
        let store = server.store();
        blocking(move || BundleEngine::open(store, StorageId::parse("storage-r2").unwrap()))
            .await
            .unwrap();

        let marker = server
            .objects
            .lock()
            .expect("test S3 lock")
            .get(".mallard/_storage.json")
            .cloned()
            .expect("Mallard storage marker");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&marker).unwrap(),
            serde_json::json!({
                "format": "mallard-storage",
                "layout_version": 1,
            })
        );

        let store = server.store();
        blocking(move || BundleEngine::open(store, StorageId::parse("storage-r2").unwrap()))
            .await
            .unwrap();
        assert_eq!(
            server
                .objects
                .lock()
                .expect("test S3 lock")
                .get(".mallard/_storage.json")
                .cloned()
                .unwrap(),
            marker
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn immutable_metadata_staging_and_cas_preserve_service_etags() {
        let server = TestS3::start().await;
        let store = server.store();
        let immutable = ObjectKey::parse(
            ".mallard/v1/repositories/0123456789abcdef0123456789abcdef/_uploads/a/file",
        )
        .unwrap();
        let store_for_write = store.clone();
        let immutable_for_write = immutable.clone();
        assert_eq!(
            blocking(move || store_for_write.put_immutable(&immutable_for_write, b"one"))
                .await
                .unwrap(),
            ImmutablePutOutcome::Written
        );
        let store_for_repeat = store.clone();
        let immutable_for_repeat = immutable.clone();
        assert_eq!(
            blocking(move || store_for_repeat.put_immutable(&immutable_for_repeat, b"one"))
                .await
                .unwrap(),
            ImmutablePutOutcome::AlreadyPresent
        );
        let store_for_collision = store.clone();
        let immutable_for_collision = immutable.clone();
        assert!(blocking(
            move || store_for_collision.put_immutable(&immutable_for_collision, b"two")
        )
        .await
        .is_err());

        let store_for_metadata = store.clone();
        let immutable_for_metadata = immutable.clone();
        let found = blocking(move || store_for_metadata.get_metadata(&immutable_for_metadata, 3))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.bytes, b"one");
        assert_eq!(found.etag, sha256_bytes(b"one"));

        let store_for_stage = store.clone();
        let immutable_for_stage = immutable.clone();
        let staged = blocking(move || {
            store_for_stage.stage_verified(
                &immutable_for_stage,
                &ExpectedObject {
                    size: 3,
                    sha256: sha256_bytes(b"one"),
                },
            )
        })
        .await
        .unwrap();
        let StageOutcome::Verified(staged) = staged else {
            panic!("immutable body should verify")
        };
        assert_eq!(staged.read_bounded(3, "test object").unwrap(), b"one");
        assert_eq!(staged.etag(), sha256_bytes(b"one"));

        let head = ObjectKey::parse(
            ".mallard/v1/repositories/0123456789abcdef0123456789abcdef/_head.json",
        )
        .unwrap();
        let store_for_create = store.clone();
        let head_for_create = head.clone();
        let created = blocking(move || {
            store_for_create.compare_and_swap(
                &head_for_create,
                &CasExpectation::Absent,
                b"head-one",
            )
        })
        .await
        .unwrap();
        let CasOutcome::Written { etag } = created else {
            panic!("head create should succeed")
        };
        assert_eq!(etag, sha256_bytes(b"head-one"));

        let store_for_stale = store.clone();
        let head_for_stale = head.clone();
        let stale = blocking(move || {
            store_for_stale.compare_and_swap(
                &head_for_stale,
                &CasExpectation::Match("stale".to_string()),
                b"wrong",
            )
        })
        .await
        .unwrap();
        assert_eq!(
            stale,
            CasOutcome::Conflict {
                current_etag: Some(etag.clone())
            }
        );

        let store_for_update = store.clone();
        let head_for_update = head.clone();
        let updated = blocking(move || {
            store_for_update.compare_and_swap(
                &head_for_update,
                &CasExpectation::Match(etag),
                b"head-two",
            )
        })
        .await
        .unwrap();
        assert_eq!(
            updated,
            CasOutcome::Written {
                etag: sha256_bytes(b"head-two")
            }
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exact_object_delete_is_idempotent() {
        let server = TestS3::start().await;
        let key = test_upload_key("delete-me");
        server.seed(key.as_str().to_string(), b"value".to_vec());
        let store = server.store();
        let first_store = store.clone();
        let first_key = key.clone();

        blocking(move || first_store.delete(&first_key))
            .await
            .unwrap();
        assert!(!server
            .objects
            .lock()
            .expect("test S3 lock")
            .contains_key(key.as_str()));
        blocking(move || store.delete(&key)).await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn metadata_get_streams_chunks_and_enforces_the_caller_bound() {
        let server = TestS3::start().await;
        let key = test_upload_key("metadata.json");
        server.seed_streaming(
            key.as_str().to_string(),
            vec![b"{\"a".to_vec(), b"\":1}".to_vec()],
            None,
            false,
        );
        let store = server.store();

        let store_for_get = store.clone();
        let key_for_get = key.clone();
        let metadata = blocking(move || store_for_get.get_metadata(&key_for_get, 7))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(metadata.bytes, b"{\"a\":1}");
        assert_eq!(metadata.etag, sha256_bytes(b"{\"a\":1}"));

        let store_for_bound = store.clone();
        let key_for_bound = key.clone();
        let error = blocking(move || store_for_bound.get_metadata(&key_for_bound, 6))
            .await
            .unwrap_err();
        assert!(error.contains("exceeds its read limit"), "{error}");

        let missing = test_upload_key("missing.json");
        assert!(blocking(move || store.get_metadata(&missing, 32))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn verified_staging_handles_streamed_size_and_digest_boundaries() {
        let server = TestS3::start().await;
        let exact = test_upload_key("exact");
        server.seed_streaming(
            exact.as_str().to_string(),
            vec![b"ab".to_vec(), b"cde".to_vec()],
            None,
            false,
        );
        let declared_mismatch = test_upload_key("declared-mismatch");
        server.seed_streaming(
            declared_mismatch.as_str().to_string(),
            vec![b"abcde".to_vec()],
            Some(6),
            false,
        );
        let overrun = test_upload_key("overrun");
        server.seed_streaming(
            overrun.as_str().to_string(),
            vec![b"abc".to_vec(), b"d".to_vec()],
            None,
            false,
        );
        let short = test_upload_key("short");
        server.seed_streaming(
            short.as_str().to_string(),
            vec![b"ab".to_vec()],
            None,
            false,
        );
        let wrong_digest = test_upload_key("wrong-digest");
        server.seed_streaming(
            wrong_digest.as_str().to_string(),
            vec![b"abc".to_vec()],
            None,
            false,
        );
        let empty = test_upload_key("empty");
        server.seed_streaming(empty.as_str().to_string(), Vec::new(), None, false);
        let store = server.store();

        let store_for_exact = store.clone();
        let exact_for_stage = exact.clone();
        let exact_outcome = blocking(move || {
            store_for_exact.stage_verified(
                &exact_for_stage,
                &ExpectedObject {
                    size: 5,
                    sha256: sha256_bytes(b"abcde"),
                },
            )
        })
        .await
        .unwrap();
        let StageOutcome::Verified(exact_body) = exact_outcome else {
            panic!("chunked exact body should verify")
        };
        assert_eq!(
            exact_body.read_bounded(5, "streamed test body").unwrap(),
            b"abcde"
        );
        assert_eq!(exact_body.etag(), sha256_bytes(b"abcde"));

        for key in [declared_mismatch, overrun, short] {
            let store_for_stage = store.clone();
            let outcome = blocking(move || {
                store_for_stage.stage_verified(
                    &key,
                    &ExpectedObject {
                        size: 3,
                        sha256: sha256_bytes(b"abc"),
                    },
                )
            })
            .await
            .unwrap();
            assert!(matches!(outcome, StageOutcome::Mismatch));
        }

        let store_for_digest = store.clone();
        let digest_outcome = blocking(move || {
            store_for_digest.stage_verified(
                &wrong_digest,
                &ExpectedObject {
                    size: 3,
                    sha256: sha256_bytes(b"xyz"),
                },
            )
        })
        .await
        .unwrap();
        assert!(matches!(digest_outcome, StageOutcome::Mismatch));

        let empty_outcome = blocking(move || {
            store.stage_verified(
                &empty,
                &ExpectedObject {
                    size: 0,
                    sha256: sha256_bytes(b""),
                },
            )
        })
        .await
        .unwrap();
        let StageOutcome::Verified(empty_body) = empty_outcome else {
            panic!("empty streamed body should verify")
        };
        assert_eq!(empty_body.read_bounded(0, "empty test body").unwrap(), b"");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn streamed_staging_uses_an_inactivity_timeout_and_bulk_preserves_order() {
        let server = TestS3::start().await;
        let stalled = test_upload_key("stalled");
        server.seed_streaming(
            stalled.as_str().to_string(),
            vec![b"a".to_vec()],
            None,
            true,
        );
        let mut store = server.store();
        store.request_timeout = Duration::from_millis(40);
        let store_for_stall = store.clone();
        let error = blocking(move || {
            store_for_stall.stage_verified(
                &stalled,
                &ExpectedObject {
                    size: 2,
                    sha256: sha256_bytes(b"ab"),
                },
            )
        })
        .await
        .unwrap_err();
        assert!(error.contains("inactivity"), "{error}");

        let requests = (0..8)
            .map(|index| {
                let key = test_upload_key(&format!("bulk-{index}"));
                let bytes = format!("payload-{index}").into_bytes();
                server.seed_streaming(
                    key.as_str().to_string(),
                    vec![bytes[..3].to_vec(), bytes[3..].to_vec()],
                    None,
                    false,
                );
                (
                    key,
                    ExpectedObject {
                        size: bytes.len() as u64,
                        sha256: sha256_bytes(&bytes),
                    },
                    bytes,
                )
            })
            .collect::<Vec<_>>();
        let stage_requests = requests
            .iter()
            .map(|(key, expected, _)| (key.clone(), expected.clone()))
            .collect::<Vec<_>>();
        let outcomes = blocking(move || store.stage_many_verified(&stage_requests))
            .await
            .unwrap();
        assert_eq!(outcomes.len(), requests.len());
        for (outcome, (_, _, expected_bytes)) in outcomes.into_iter().zip(requests) {
            let StageOutcome::Verified(body) = outcome else {
                panic!("bulk body should verify")
            };
            assert_eq!(
                body.read_bounded(expected_bytes.len() as u64, "bulk test body")
                    .unwrap(),
                expected_bytes
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn put_many_bulk_uploads_and_adopts_existing_objects() {
        let server = TestS3::start().await;
        let store = server.store();
        let repository = ".mallard/v1/repositories/0123456789abcdef0123456789abcdef";
        let existing =
            ObjectKey::parse(format!("{}/_objects/{}", repository, "b".repeat(64))).unwrap();
        server.seed(existing.as_str().to_string(), b"already there".to_vec());
        let fresh_one =
            ObjectKey::parse(format!("{}/_objects/{}", repository, "c".repeat(64))).unwrap();
        let fresh_two =
            ObjectKey::parse(format!("{}/_objects/{}", repository, "d".repeat(64))).unwrap();
        // More objects than the concurrency window, exercising the semaphore.
        let filler: Vec<ObjectKey> = (0..8)
            .map(|index| {
                ObjectKey::parse(format!("{}/_objects/{:064x}", repository, index)).unwrap()
            })
            .collect();
        let mut objects = vec![
            (fresh_one.clone(), Bytes::from_static(b"payload one")),
            (existing.clone(), Bytes::from_static(b"already there")),
            (fresh_two.clone(), Bytes::from_static(b"payload two")),
        ];
        for (index, key) in filler.iter().enumerate() {
            objects.push((
                key.clone(),
                Bytes::from(format!("filler {}", index).into_bytes()),
            ));
        }
        let store_for_bulk = store.clone();
        let outcomes = blocking(move || store_for_bulk.put_many(objects))
            .await
            .unwrap();
        assert_eq!(outcomes.len(), 11);
        assert_eq!(outcomes[0], ImmutablePutOutcome::Written);
        assert_eq!(outcomes[1], ImmutablePutOutcome::AlreadyPresent);
        assert_eq!(outcomes[2], ImmutablePutOutcome::Written);
        assert!(outcomes[3..]
            .iter()
            .all(|outcome| *outcome == ImmutablePutOutcome::Written));
        {
            let stored = server.objects.lock().expect("test S3 lock");
            assert_eq!(
                stored.get(fresh_one.as_str()).map(Vec::as_slice),
                Some(&b"payload one"[..])
            );
            assert_eq!(
                stored.get(fresh_two.as_str()).map(Vec::as_slice),
                Some(&b"payload two"[..])
            );
            for (index, key) in filler.iter().enumerate() {
                assert_eq!(
                    stored.get(key.as_str()).cloned(),
                    Some(format!("filler {}", index).into_bytes())
                );
            }
        }

        // Conflicting bytes at an existing key fail the batch.
        let conflict = vec![(existing, Bytes::from_static(b"different bytes"))];
        let store_for_conflict = store.clone();
        assert!(blocking(move || store_for_conflict.put_many(conflict))
            .await
            .is_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_uses_stable_key_cursors_across_s3_pages() {
        let server = TestS3::start().await;
        for index in 0..1_005 {
            server.seed(
                format!(".mallard/v1/repositories/{:032x}/_tag.json", index),
                index.to_string().into_bytes(),
            );
        }
        // A neighboring key must not leak from the exact prefix.
        server.seed(
            ".mallard/v1/repositories-neighbor/value".to_string(),
            b"no".to_vec(),
        );
        let store = server.store();
        let prefix = ObjectPrefix::parse(".mallard/v1/repositories").unwrap();
        let store_for_first = store.clone();
        let prefix_for_first = prefix.clone();
        let first = blocking(move || store_for_first.list(&prefix_for_first, None, 1_001))
            .await
            .unwrap();
        assert_eq!(first.keys.len(), 1_001);
        let cursor = first.next_cursor.expect("more than 1001 objects");

        let store_for_second = store.clone();
        let prefix_for_second = prefix.clone();
        let second =
            blocking(move || store_for_second.list(&prefix_for_second, Some(&cursor), 1_001))
                .await
                .unwrap();
        assert_eq!(second.keys.len(), 4);
        assert!(second.next_cursor.is_none());
        assert!(second
            .keys
            .windows(2)
            .all(|pair| pair[0].as_str() < pair[1].as_str()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r2_capability_probe_verifies_conditional_writes_and_cleans_up() {
        let server = TestS3::start().await;
        let store = server.store();
        let receipt =
            blocking(move || store.probe_r2_capabilities("r2setup-0123456789abcdef01234567"))
                .await
                .unwrap();

        assert_eq!(receipt.setup_id(), "r2setup-0123456789abcdef01234567");
        assert!(server.objects.lock().expect("test S3 lock").is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r2_capability_probe_waits_for_fresh_credentials_before_writing() {
        let server = TestS3::start().await;
        server.fail_list_authorization(2);
        let store = server.store();
        let receipt =
            blocking(move || store.probe_r2_capabilities("r2setup-0923456789abcdef01234567"))
                .await
                .unwrap();

        assert_eq!(receipt.setup_id(), "r2setup-0923456789abcdef01234567");
        assert_eq!(server.list_authorization_failures.load(Ordering::SeqCst), 0);
        assert!(server.objects.lock().expect("test S3 lock").is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r2_capability_probe_rejects_stores_that_accept_stale_etags() {
        let server = TestS3::start().await;
        server.ignore_conditions();
        let store = server.store();
        let error =
            blocking(move || store.probe_r2_capabilities("r2setup-1123456789abcdef01234567"))
                .await
                .unwrap_err();

        assert_eq!(error.kind(), R2CapabilityProbeErrorKind::Remote);
        assert_eq!(
            error.stage(),
            R2CapabilityProbeStage::StaleConditionalRejection
        );
        assert!(server.objects.lock().expect("test S3 lock").is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r2_capability_probe_reports_only_a_setup_id_when_cleanup_fails() {
        let server = TestS3::start().await;
        server.fail_delete();
        let store = server.store();
        let error =
            blocking(move || store.probe_r2_capabilities("r2setup-2123456789abcdef01234567"))
                .await
                .unwrap_err();

        assert_eq!(error.kind(), R2CapabilityProbeErrorKind::CleanupRequired);
        assert_eq!(error.setup_id(), "r2setup-2123456789abcdef01234567");
        let display = error.to_string();
        assert!(display.contains("r2setup-2123456789abcdef01234567"));
        assert!(!display.contains(R2_SETUP_PROBE_PREFIX));
        assert_eq!(server.objects.lock().expect("test S3 lock").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn r2_probe_reconciliation_deletes_only_the_journaled_setup_key() {
        let server = TestS3::start().await;
        let setup_id = "r2setup-3123456789abcdef01234567";
        let probe_key = setup_probe_key(setup_id).unwrap();
        let unrelated = ".mallard/v1/_setup_probes/r2setup-4123456789abcdef01234567";
        server.seed(probe_key, b"stale probe".to_vec());
        server.seed(unrelated.to_string(), b"keep".to_vec());
        server.fail_list_authorization(2);
        let store = server.store();
        blocking(move || store.reconcile_r2_capability_probe(setup_id))
            .await
            .unwrap();

        let objects = server.objects.lock().expect("test S3 lock");
        assert_eq!(objects.len(), 1);
        assert_eq!(
            objects.get(unrelated).map(Vec::as_slice),
            Some(&b"keep"[..])
        );
    }

    #[test]
    fn r2_capability_probe_rejects_unscoped_probe_identifiers() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let credentials = Credentials::new("test", "test", None, None, "bundle-tests");
        let config = S3Config::builder()
            .credentials_provider(credentials)
            .region(Region::new("us-east-1"))
            .endpoint_url("http://127.0.0.1:9")
            .force_path_style(true)
            .behavior_version_latest()
            .build();
        let store = S3ObjectStore::from_client(
            S3Client::from_conf(config),
            BUCKET,
            runtime.handle().clone(),
        )
        .unwrap();
        let error = store.probe_r2_capabilities("../../foreign").unwrap_err();
        assert_eq!(error.stage(), R2CapabilityProbeStage::Validate);
    }

    #[test]
    fn rejects_unsafe_cas_etags_and_foreign_cursors() {
        assert!(validate_expected_etag("").is_err());
        assert!(validate_expected_etag("bad\"etag").is_err());
        assert!(validate_expected_etag("good-etag").is_ok());
        let prefix =
            ObjectPrefix::parse(".mallard/v1/repositories/0123456789abcdef0123456789abcdef")
                .unwrap();
        let foreign =
            ObjectKey::parse(".mallard/v1/repositories/ffffffffffffffffffffffffffffffff/_tag.json")
                .unwrap();
        assert!(!key_is_under_prefix(&foreign, &prefix));
    }

    async fn test_s3_request(
        request: Request<Incoming>,
        objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        get_behaviors: Arc<Mutex<BTreeMap<String, GetBehavior>>>,
        honor_conditions: Arc<AtomicBool>,
        fail_delete: Arc<AtomicBool>,
        list_authorization_failures: Arc<AtomicUsize>,
    ) -> Result<Response<TestBody>, Infallible> {
        let method = request.method().clone();
        let path = request.uri().path().trim_start_matches('/').to_string();
        let query = request.uri().query().unwrap_or("").to_string();
        let if_match = request
            .headers()
            .get("if-match")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim_matches('"').to_string());
        let if_none_match = request
            .headers()
            .get("if-none-match")
            .and_then(|value| value.to_str().ok())
            == Some("*");
        let body = request
            .into_body()
            .collect()
            .await
            .expect("collect test request")
            .to_bytes()
            .to_vec();
        let (bucket, key) = path.split_once('/').unwrap_or((&path, ""));
        if bucket != BUCKET {
            return Ok(xml_error(StatusCode::NOT_FOUND, "NoSuchBucket"));
        }

        if method == Method::GET && key.is_empty() && query.contains("list-type=2") {
            if list_authorization_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Ok(xml_error(StatusCode::FORBIDDEN, "AccessDenied"));
            }
            return Ok(list_response(&query, &objects));
        }
        if method == Method::GET {
            if let Some(behavior) = get_behaviors
                .lock()
                .expect("test S3 lock")
                .get(key)
                .cloned()
            {
                return Ok(streaming_object_response(StatusCode::OK, behavior));
            }
        }

        let mut objects = objects.lock().expect("test S3 lock");
        match method {
            Method::GET => Ok(match objects.get(key) {
                Some(bytes) => object_response(StatusCode::OK, bytes.clone(), Some(bytes)),
                None => xml_error(StatusCode::NOT_FOUND, "NoSuchKey"),
            }),
            Method::PUT => {
                let current = objects.get(key);
                let precondition_failed = honor_conditions.load(Ordering::SeqCst)
                    && ((if_none_match && current.is_some())
                        || if_match.as_ref().is_some_and(|expected| {
                            current.map(|bytes| sha256_bytes(bytes)).as_deref()
                                != Some(expected.as_str())
                        }));
                if precondition_failed {
                    return Ok(xml_error(
                        StatusCode::PRECONDITION_FAILED,
                        "PreconditionFailed",
                    ));
                }
                objects.insert(key.to_string(), body.clone());
                Ok(object_response(StatusCode::OK, Vec::new(), Some(&body)))
            }
            Method::HEAD => Ok(match objects.get(key) {
                Some(bytes) => object_response(StatusCode::OK, Vec::new(), Some(bytes)),
                None => xml_error(StatusCode::NOT_FOUND, "NoSuchKey"),
            }),
            Method::DELETE => {
                if fail_delete.load(Ordering::SeqCst) {
                    return Ok(xml_error(StatusCode::INTERNAL_SERVER_ERROR, "DeleteFailed"));
                }
                objects.remove(key);
                Ok(object_response(StatusCode::NO_CONTENT, Vec::new(), None))
            }
            _ => Ok(xml_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "MethodNotAllowed",
            )),
        }
    }

    fn list_response(
        query: &str,
        objects: &Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    ) -> Response<TestBody> {
        let prefix = query_value(query, "prefix").unwrap_or_default();
        let start_after =
            query_value(query, "start-after").or_else(|| query_value(query, "continuation-token"));
        let max_keys = query_value(query, "max-keys")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(S3_MAX_KEYS);
        let objects = objects.lock().expect("test S3 lock");
        let matching = objects
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .filter(|key| start_after.as_ref().is_none_or(|after| *key > after))
            .cloned()
            .collect::<Vec<_>>();
        let truncated = matching.len() > max_keys;
        let page = matching.into_iter().take(max_keys).collect::<Vec<_>>();
        let next = truncated.then(|| page.last().expect("truncated page has a key").clone());
        let mut xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{}</Name><Prefix>{}</Prefix><IsTruncated>{}</IsTruncated><KeyCount>{}</KeyCount>",
            BUCKET,
            prefix,
            truncated,
            page.len()
        );
        for key in &page {
            let bytes = &objects[key];
            xml.push_str(&format!(
                "<Contents><Key>{}</Key><Size>{}</Size><ETag>&quot;{}&quot;</ETag><LastModified>2026-01-01T00:00:00.000Z</LastModified><StorageClass>STANDARD</StorageClass></Contents>",
                key,
                bytes.len(),
                sha256_bytes(bytes)
            ));
        }
        if let Some(next) = next {
            xml.push_str(&format!(
                "<NextContinuationToken>{}</NextContinuationToken>",
                next
            ));
        }
        xml.push_str("</ListBucketResult>");
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/xml")
            .header("content-length", xml.len())
            .body(TestBody::full(xml))
            .unwrap()
    }

    fn object_response(
        status: StatusCode,
        body: Vec<u8>,
        etag_bytes: Option<&[u8]>,
    ) -> Response<TestBody> {
        let mut response = Response::builder()
            .status(status)
            .header("content-length", body.len());
        if let Some(bytes) = etag_bytes {
            response = response.header("etag", format!("\"{}\"", sha256_bytes(bytes)));
        }
        response.body(TestBody::full(body)).unwrap()
    }

    fn streaming_object_response(status: StatusCode, behavior: GetBehavior) -> Response<TestBody> {
        let all_bytes = behavior
            .chunks
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let mut response = Response::builder()
            .status(status)
            .header("etag", format!("\"{}\"", sha256_bytes(&all_bytes)));
        if let Some(length) = behavior.declared_length {
            response = response.header("content-length", length);
        }
        response
            .body(TestBody::streaming(
                behavior.chunks,
                behavior.stall_after_chunks,
            ))
            .unwrap()
    }

    fn xml_error(status: StatusCode, code: &str) -> Response<TestBody> {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{}</Code><Message>{}</Message></Error>",
            code, code
        );
        Response::builder()
            .status(status)
            .header("content-type", "application/xml")
            .header("content-length", body.len())
            .body(TestBody::full(body))
            .unwrap()
    }

    fn query_value(query: &str, name: &str) -> Option<String> {
        query.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key == name).then(|| percent_decode(value))
        })
    }

    fn percent_decode(value: &str) -> String {
        let bytes = value.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' && index + 2 < bytes.len() {
                if let Ok(byte) = u8::from_str_radix(
                    std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                    16,
                ) {
                    decoded.push(byte);
                    index += 3;
                    continue;
                }
            }
            decoded.push(if bytes[index] == b'+' {
                b' '
            } else {
                bytes[index]
            });
            index += 1;
        }
        String::from_utf8_lossy(&decoded).into_owned()
    }
}
