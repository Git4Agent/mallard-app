//! Read-only discovery across one account's jurisdictions.
use super::*;
use crate::storage_core::{MAX_STORAGE_MARKER_BYTES, STORAGE_MARKER_KEY};
use std::collections::BTreeSet;

pub const MAX_BUCKETS: usize = 256;
const MAX_MARKER_DETAIL_CHARS: usize = 64;

pub use crate::storage_core::MarkerState;
use crate::storage_core::classify_storage_marker as classify_marker;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredBucket {
    pub name: String,
    pub creation_date: String,
    /// The jurisdiction the bucket was listed under, as Cloudflare names it.
    pub jurisdiction: String,
    pub marker: MarkerState,
    /// The layout the marker declares, or the fixed reason it could not be read.
    pub marker_detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BucketScan {
    pub buckets: Vec<DiscoveredBucket>,
    /// A page, a read, or a limit cut the scan short: what was found stands,
    /// but a Mallard storage may be missing from it.
    pub partial: bool,
    /// The listing itself stopped short, so not every bucket is here — as
    /// opposed to a bucket listed whose marker could not be read.
    pub cut: bool,
    /// The jurisdictions whose listings were read, in scan order. One the
    /// account cannot use is left out without making the scan partial.
    pub jurisdictions: Vec<String>,
}

impl BucketScan {
    pub fn validate(&self, account: &str) -> bool {
        let known = |value: &str| R2Jurisdiction::parse(value).is_some();
        self.buckets.len() <= MAX_BUCKETS
            && self.jurisdictions.len() <= R2Jurisdiction::ALL.len()
            && self.jurisdictions.iter().all(|value| known(value))
            && self.jurisdictions.iter().collect::<BTreeSet<_>>().len() == self.jurisdictions.len()
            && self.buckets.iter().all(|bucket| {
                R2Jurisdiction::parse(&bucket.jurisdiction).is_some_and(|jurisdiction| {
                    destination(account, jurisdiction, &bucket.name).is_ok()
                }) && valid_date(&bucket.creation_date)
                    && bucket.marker_detail.as_deref().is_none_or(valid_detail)
            })
            && self
                .buckets
                .iter()
                .map(|bucket| (&bucket.jurisdiction, &bucket.name))
                .collect::<BTreeSet<_>>()
                .len()
                == self.buckets.len()
    }
}

fn valid_detail(detail: &str) -> bool {
    !detail.is_empty()
        && detail.chars().count() <= MAX_MARKER_DETAIL_CHARS
        && !detail.chars().any(char::is_control)
}

/// How one jurisdiction's listing ended.
enum Listing {
    /// Every page was read.
    Covered,
    /// The account cannot list this jurisdiction: Cloudflare refused the first
    /// page outright, which is how a jurisdiction the account is not entitled
    /// to answers. Nothing was skipped that the account could reach.
    Unavailable,
    /// A page, a read, or a limit stopped the listing short.
    Cut,
}

impl R2SetupClient {
    async fn discovery_get(
        &self,
        url: reqwest::Url,
        jurisdiction: R2Jurisdiction,
        limit: usize,
    ) -> Result<(u16, Zeroizing<Vec<u8>>), SetupError> {
        let authorization = Zeroizing::new(format!("Bearer {}", self.token.expose()));
        let mut header = reqwest::header::HeaderValue::from_str(&authorization)
            .map_err(|_| SetupError::Local)?;
        header.set_sensitive(true);
        let mut response = self
            .client
            .get(url)
            .header("Authorization", header)
            .header("cf-r2-jurisdiction", jurisdiction.as_str())
            .send()
            .await
            .map_err(|_| SetupError::Rejected)?;
        let status = response.status().as_u16();
        if matches!(status, 401 | 403) {
            return Err(SetupError::Permission);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| SetupError::Rejected)? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(SetupError::Rejected);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((status, bytes))
    }

    /// One marker read: what it said, and the layout or the fixed reason.
    async fn marker(
        &self,
        account: &str,
        jurisdiction: R2Jurisdiction,
        bucket: &str,
    ) -> (MarkerState, Option<String>) {
        if destination(account, jurisdiction, bucket).is_err() {
            return (MarkerState::Invalid, None);
        }
        let Ok(url) = reqwest::Url::parse(&format!(
            "{}/accounts/{account}/r2/buckets/{bucket}/objects/{STORAGE_MARKER_KEY}",
            self.api
        )) else {
            return (MarkerState::Unreadable, Some("read failed".into()));
        };
        match self
            .discovery_get(url, jurisdiction, MAX_STORAGE_MARKER_BYTES as usize)
            .await
        {
            Ok((404, _)) => (MarkerState::Missing, None),
            Ok((200, bytes)) => classify_marker(&bytes),
            Err(SetupError::Permission) => (MarkerState::Unreadable, Some("access denied".into())),
            _ => (MarkerState::Unreadable, Some("read failed".into())),
        }
    }

    pub async fn discover(&self, account: &str) -> Result<BucketScan, SetupError> {
        destination(account, R2Jurisdiction::Default, "mallard-check")?;
        let mut scan = BucketScan {
            buckets: Vec::new(),
            partial: false,
            cut: false,
            jurisdictions: Vec::new(),
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        for jurisdiction in R2Jurisdiction::ALL {
            match self
                .list_jurisdiction(account, jurisdiction, deadline, &mut scan)
                .await?
            {
                Listing::Covered => scan.jurisdictions.push(jurisdiction.as_str().into()),
                Listing::Unavailable => {}
                Listing::Cut => {
                    scan.partial = true;
                    scan.cut = true;
                    scan.jurisdictions.push(jurisdiction.as_str().into());
                }
            }
        }
        Ok(scan)
    }

    /// Page through one jurisdiction's buckets and read each marker. The
    /// default jurisdiction is the account itself: a refused first page fails
    /// the scan. Any other jurisdiction is optional to an account, so its
    /// refused first page only leaves it out.
    async fn list_jurisdiction(
        &self,
        account: &str,
        jurisdiction: R2Jurisdiction,
        deadline: tokio::time::Instant,
        scan: &mut BucketScan,
    ) -> Result<Listing, SetupError> {
        let mut cursor = String::new();
        let mut cursors = BTreeSet::new();
        let mut names = BTreeSet::new();
        for page in 0..10 {
            let mut url =
                reqwest::Url::parse(&format!("{}/accounts/{account}/r2/buckets", self.api))
                    .map_err(|_| SetupError::Local)?;
            url.query_pairs_mut().append_pair("per_page", "100");
            if !cursor.is_empty() {
                url.query_pairs_mut().append_pair("cursor", &cursor);
            }
            let response = tokio::time::timeout_at(
                deadline,
                self.discovery_get(url, jurisdiction, 256 * 1024),
            )
            .await;
            let first_page = page == 0;
            let optional = jurisdiction != R2Jurisdiction::Default;
            let value: Value = match response {
                Ok(Ok((200, bytes))) => {
                    serde_json::from_slice(&bytes).map_err(|_| SetupError::Rejected)?
                }
                Ok(Err(SetupError::Permission)) | Ok(Ok((400..=499, _)))
                    if first_page && optional =>
                {
                    return Ok(Listing::Unavailable);
                }
                Ok(Err(error)) if first_page && !optional => return Err(error),
                _ => return Ok(Listing::Cut),
            };
            let Some(buckets) = value["result"]["buckets"]
                .as_array()
                .filter(|b| b.len() <= 100 && value["success"] == true)
            else {
                return Ok(Listing::Cut);
            };
            for bucket in buckets {
                let name = bucket["name"].as_str().unwrap_or_default();
                let date = bucket["creation_date"].as_str().unwrap_or_default();
                if destination(account, jurisdiction, name).is_err()
                    || !valid_date(date)
                    || bucket
                        .get("jurisdiction")
                        .is_some_and(|v| v != jurisdiction.as_str())
                {
                    scan.partial = true;
                    scan.cut = true;
                    continue;
                }
                if !names.insert(name.to_string()) {
                    continue;
                }
                if scan.buckets.len() == MAX_BUCKETS {
                    return Ok(Listing::Cut);
                }
                let (marker, marker_detail) =
                    tokio::time::timeout_at(deadline, self.marker(account, jurisdiction, name))
                        .await
                        .unwrap_or((MarkerState::Unreadable, Some("timed out".into())));
                scan.partial |= marker == MarkerState::Unreadable;
                scan.buckets.push(DiscoveredBucket {
                    name: name.into(),
                    creation_date: date.into(),
                    jurisdiction: jurisdiction.as_str().into(),
                    marker,
                    marker_detail,
                });
            }
            let next = &value["result_info"]["cursor"];
            if next.is_null() || next == "" {
                return Ok(Listing::Covered);
            }
            let Some(next) = next.as_str().filter(|c| c.len() <= 16 * 1024) else {
                return Ok(Listing::Cut);
            };
            if !cursors.insert(next.to_string())
                || page == 9
                || scan.buckets.len() == MAX_BUCKETS
                || tokio::time::Instant::now() >= deadline
            {
                return Ok(Listing::Cut);
            }
            cursor = next.into();
        }
        Ok(Listing::Cut)
    }

    /// Reject a changed, deleted, or unrecognized destination before and after consent.
    pub async fn verify_existing(
        &self,
        account: &str,
        jurisdiction: R2Jurisdiction,
        bucket: &str,
        expected_date: &str,
    ) -> Result<R2Destination, SetupError> {
        let destination = destination(account, jurisdiction, bucket)?;
        if !valid_date(expected_date)
            || self.lookup(account, jurisdiction, bucket).await?.as_deref() != Some(expected_date)
            || self.marker(account, jurisdiction, bucket).await.0 != MarkerState::Storage
        {
            return Err(SetupError::Identity);
        }
        Ok(destination)
    }
}
