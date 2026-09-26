//! Account-bound R2 provisioning. Credentials remain in memory, and nothing
//! about an attempt is recorded locally: every attempt is a fresh attempt.
use crate::storage_names::{r2_bucket, select_word};
use crate::storage_providers::r2::{parse_r2_api_url, R2Destination, R2Jurisdiction};
use mallard_domain::SecretValue;
use reqwest::{Client, Method};
use serde_json::{json, Value};
use std::time::Duration;
use zeroize::Zeroizing;

pub mod discovery;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    Local,
    Identity,
    Authentication,
    Permission,
    R2NotEnabled,
    RateLimited,
    Unavailable,
    Rejected,
    Uncertain,
    Taken,
}

impl SetupError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Local => "r2_setup_client_failed",
            Self::Identity => "r2_setup_identity_mismatch",
            Self::Authentication => "r2_setup_authentication_failed",
            Self::Permission => "r2_setup_permission_denied",
            Self::R2NotEnabled => "r2_setup_not_enabled",
            Self::RateLimited => "r2_setup_rate_limited",
            Self::Unavailable => "r2_setup_unavailable",
            Self::Rejected => "r2_setup_request_rejected",
            Self::Uncertain => "r2_setup_outcome_unknown",
            Self::Taken => "r2_bucket_name_taken",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Local => "Mallard could not start R2 setup.",
            Self::Identity => "The Cloudflare account or bucket does not match this setup.",
            Self::Authentication => "Cloudflare could not verify the sign-in. Select Change login, then sign in again.",
            Self::Permission => "Cloudflare denied R2 setup access. Check the account permissions and Wrangler login.",
            Self::R2NotEnabled => "R2 is not enabled for this account. Complete R2 setup in the Cloudflare dashboard, then try again.",
            Self::RateLimited => "Cloudflare received too many requests. Wait, then try again.",
            Self::Unavailable => "Could not reach Cloudflare or its service is temporarily unavailable. Check your connection, then try again.",
            Self::Rejected => "Cloudflare rejected the R2 setup request.",
            Self::Uncertain => "Cloudflare did not confirm whether the bucket was created. Check the account's buckets, then try again or choose Use existing.",
            Self::Taken => "A bucket with this name already exists in the account. Choose another name.",
        }
    }
}

/// Read only the provider's error code; never expose its response text.
fn response_error(status: u16, bytes: &[u8]) -> Option<SetupError> {
    if (400..500).contains(&status) {
        if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
            if value["success"] == false
                && value["errors"].as_array().is_some_and(|errors| {
                    errors
                        .iter()
                        .any(|error| error["code"].as_u64() == Some(10042))
                })
            {
                return Some(SetupError::R2NotEnabled);
            }
        }
    }
    match status {
        401 => Some(SetupError::Authentication),
        403 => Some(SetupError::Permission),
        429 => Some(SetupError::RateLimited),
        408 | 500..=599 => Some(SetupError::Unavailable),
        _ => None,
    }
}

/// A readable name for a new bucket, `mallard-<word>`. Whether it is free is
/// only known at creation, when the account's own buckets answer.
pub fn propose_bucket_name() -> Result<String, SetupError> {
    select_word(&[])
        .map(|word| r2_bucket(&word))
        .map_err(|_| SetupError::Local)
}

/// The canonical destination for a bucket, validated as the adapter would.
pub fn destination(
    account: &str,
    jurisdiction: R2Jurisdiction,
    bucket: &str,
) -> Result<R2Destination, SetupError> {
    parse_r2_api_url(&format!("{}/{bucket}", jurisdiction.endpoint(account)))
        .map_err(|_| SetupError::Identity)
}

fn valid_date(date: &str) -> bool {
    date.len() <= 64 && chrono::DateTime::parse_from_rfc3339(date).is_ok()
}

pub struct R2SetupClient {
    client: Client,
    token: SecretValue,
    api: String,
}

impl R2SetupClient {
    pub fn new(token: SecretValue) -> Result<Self, SetupError> {
        Self::with_endpoint(token, "https://api.cloudflare.com/client/v4")
    }

    fn with_endpoint(token: SecretValue, api: &str) -> Result<Self, SetupError> {
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| SetupError::Local)?,
            token,
            api: api.into(),
        })
    }

    async fn request(
        &self,
        method: Method,
        account: &str,
        jurisdiction: R2Jurisdiction,
        bucket: &str,
    ) -> Result<(u16, Value), SetupError> {
        // All path components pass canonical R2 validation before dispatch.
        destination(account, jurisdiction, bucket)?;
        let mut url = format!("{}/accounts/{account}/r2/buckets", self.api);
        if method == Method::GET {
            url.push('/');
            url.push_str(bucket);
        }
        let authorization = Zeroizing::new(format!("Bearer {}", self.token.expose()));
        let mut header = reqwest::header::HeaderValue::from_str(&authorization)
            .map_err(|_| SetupError::Local)?;
        header.set_sensitive(true);
        let mut request = self
            .client
            .request(method.clone(), url)
            .header("Authorization", header)
            .header("cf-r2-jurisdiction", jurisdiction.as_str());
        if method == Method::POST {
            request = request.json(&json!({"name":bucket}));
        }
        // A failed read can be retried. A failed creation may have succeeded.
        let transport_error = if method == Method::GET {
            SetupError::Unavailable
        } else {
            SetupError::Uncertain
        };
        let mut response = request.send().await.map_err(|_| transport_error)?;
        let status = response.status().as_u16();
        if status == 408 || (500..600).contains(&status) {
            return Err(transport_error);
        }
        let read_error = response_error(status, &[]).unwrap_or(transport_error);
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| read_error)? {
            if bytes.len().saturating_add(chunk.len()) > 256 * 1024 {
                return Err(read_error);
            }
            bytes.extend_from_slice(&chunk);
        }
        if let Some(error) = response_error(status, &bytes) {
            return Err(error);
        }
        let value = serde_json::from_slice(&bytes).map_err(|_| transport_error)?;
        Ok((status, value))
    }

    async fn lookup(
        &self,
        account: &str,
        jurisdiction: R2Jurisdiction,
        bucket: &str,
    ) -> Result<Option<String>, SetupError> {
        let (status, value) = self
            .request(Method::GET, account, jurisdiction, bucket)
            .await?;
        if status == 404 && value["success"] == false {
            return Ok(None);
        }
        if status == 200 && value["success"] == true {
            return creation_date(&value, bucket).map(Some);
        }
        Err(SetupError::Rejected)
    }

    async fn create(
        &self,
        account: &str,
        jurisdiction: R2Jurisdiction,
        bucket: &str,
    ) -> Result<Option<String>, SetupError> {
        let (status, value) = self
            .request(Method::POST, account, jurisdiction, bucket)
            .await?;
        if (200..300).contains(&status) && value["success"] == true {
            return creation_date(&value, bucket).map(Some);
        }
        if matches!(status, 400 | 409) && value["success"] == false {
            // The management API reports duplicate names as 400. Confirm the
            // occupied name with GET; a generic error status is not a conflict.
            if self.lookup(account, jurisdiction, bucket).await?.is_some() {
                return Ok(None);
            }
            return Err(SetupError::Rejected);
        }
        if (400..500).contains(&status) && value["success"] == false {
            return Err(SetupError::Rejected);
        }
        Err(SetupError::Uncertain)
    }
}

fn creation_date(value: &Value, bucket: &str) -> Result<String, SetupError> {
    let result = &value["result"];
    let date = result["creation_date"]
        .as_str()
        .filter(|date| valid_date(date))
        .ok_or(SetupError::Uncertain)?;
    if result["name"] != bucket {
        return Err(SetupError::Uncertain);
    }
    Ok(date.into())
}

/// Create the bucket `name` in the account: look the name up, then create
/// it. A name the account already has is reported as taken and nothing is
/// made. A creation whose answer was lost is reported as uncertain; the
/// next attempt finds the bucket and reports the name taken.
pub async fn prepare_bucket(
    client: &R2SetupClient,
    account: &str,
    jurisdiction: R2Jurisdiction,
    name: &str,
) -> Result<R2Destination, SetupError> {
    // The name passes the adapter's rule before it is sent.
    let destination = destination(account, jurisdiction, name)?;
    if client.lookup(account, jurisdiction, name).await?.is_some() {
        return Err(SetupError::Taken);
    }
    match client.create(account, jurisdiction, name).await? {
        Some(_) => Ok(destination),
        // Made by someone else between the lookup and the request.
        None => Err(SetupError::Taken),
    }
}

#[cfg(test)]
mod tests;
