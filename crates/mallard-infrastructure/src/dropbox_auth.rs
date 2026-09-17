//! Dropbox public-client authorization. Credentials never appear in errors.
use crate::storage_core::{
    ConnectionEvidence, ConnectionHealth, ConnectionHealthCell, ConnectionState,
};
use mallard_domain::{now_secs, SecretValue};
use reqwest::{header::HeaderValue, Client, RequestBuilder};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

// Public client ID for the registered Dropbox app "mallard-app"; not an app secret.
pub const APP_KEY: &str = "ex00srwhe5f5m5m";
pub const CALLBACK_URI: &str = "http://127.0.0.1:53682/dropbox/callback";
pub const CALLBACK_PORT: u16 = 53682;
/// The Dropbox callback shape for the shared loopback receiver.
pub const DROPBOX_CALLBACK: crate::loopback_browser::LoopbackCallback =
    crate::loopback_browser::LoopbackCallback {
        path: "/dropbox/callback",
        result_key: "code",
        completion_path: "/dropbox/complete",
        title: "Dropbox authorization",
        received_message: "Dropbox authorization received. Return to Mallard and wait for setup to finish.",
        failed_message: "Dropbox authorization was not completed. Return to Mallard.",
        errors: crate::loopback_browser::LoopbackErrors {
            invalid: (
                "dropbox_callback_invalid",
                "Dropbox authorization returned an invalid callback. Run setup again.",
            ),
            unavailable: (
                "dropbox_callback_unavailable",
                "Mallard could not bind the Dropbox callback port. Close the other setup attempt and try again.",
            ),
            browser_failed: (
                "dropbox_browser_failed",
                "Mallard could not open the Dropbox authorization page.",
            ),
            cancelled: ("dropbox_setup_cancelled", "Dropbox setup was cancelled."),
            denied: (
                "dropbox_consent_denied",
                "Dropbox access was not approved. Setup stopped.",
            ),
            timeout: (
                "dropbox_authorization_timeout",
                "Dropbox authorization timed out. Run setup again when you are ready.",
            ),
        },
    };
pub const STORAGE_SCOPES: &str =
    "account_info.read files.metadata.read files.content.read files.content.write";
pub const SETUP_SCOPES: &str =
    "account_info.read files.metadata.read files.content.read files.content.write sharing.write";
pub const DISCOVERY_SCOPES: &str =
    "account_info.read files.metadata.read files.content.read files.content.write sharing.read";
const RESPONSE_LIMIT: usize = 1024 * 1024;

pub struct AuthorizationRequest {
    pub url: url::Url,
    pub state: Zeroizing<String>,
    pub verifier: Zeroizing<String>,
}
impl AuthorizationRequest {
    pub fn new(scopes: &str) -> Result<Self, AuthError> {
        Self::for_login(scopes, false)
    }

    pub fn for_login(scopes: &str, switch_account: bool) -> Result<Self, AuthError> {
        use sha2::{Digest, Sha256};
        let random = || -> Result<Zeroizing<String>, AuthError> {
            let mut bytes = Zeroizing::new([0u8; 32]);
            getrandom::fill(bytes.as_mut()).map_err(|_| AuthError::Local)?;
            Ok(Zeroizing::new(mallard_domain::hex_lower(bytes.as_ref())))
        };
        let state = random()?;
        let verifier = random()?;
        let challenge = aws_smithy_types::base64::encode(Sha256::digest(verifier.as_bytes()))
            .trim_end_matches('=')
            .replace('+', "-")
            .replace('/', "_");
        let mut url = url::Url::parse("https://www.dropbox.com/oauth2/authorize")
            .map_err(|_| AuthError::Local)?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", APP_KEY),
            ("response_type", "code"),
            ("token_access_type", "offline"),
            ("redirect_uri", CALLBACK_URI),
            ("scope", scopes),
            ("state", state.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
        ]);
        if switch_account {
            url.query_pairs_mut()
                .append_pair("force_reauthentication", "true");
        }
        Ok(Self {
            url,
            state,
            verifier,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    Reconnect,
    Permission,
    Invalid,
    Remote,
    Local,
}
impl AuthError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Reconnect => "dropbox_reconnect_required",
            Self::Permission => "dropbox_permission_denied",
            Self::Invalid => "dropbox_response_invalid",
            Self::Remote => "dropbox_request_failed",
            Self::Local => "dropbox_client_failed",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Reconnect => "Dropbox authorization expired or was revoked. Reconnect Dropbox in storage settings.",
            Self::Permission => "Dropbox access is denied. Check the app permissions and account policy.",
            Self::Invalid => "Dropbox returned an invalid authorization or setup response.",
            Self::Remote => "The Dropbox request did not complete. No mutation was retried.",
            Self::Local => "Mallard could not start the Dropbox connection.",
        }
    }
}

/// An authorization failure with the evidence the response carried: the
/// endpoint literal, the status, the fixed error identifier, and the
/// request ID. Never the body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthFailure {
    pub error: AuthError,
    pub evidence: Option<ConnectionEvidence>,
}
impl From<AuthFailure> for AuthError {
    fn from(failure: AuthFailure) -> Self {
        failure.error
    }
}

/// The fixed identifiers Dropbox's `AuthError` union can carry under HTTP
/// 401. Nothing outside this list is ever retained from a response.
pub const AUTH_ERROR_TAGS: [&str; 7] = [
    "invalid_access_token",
    "expired_access_token",
    "missing_scope",
    "user_suspended",
    "route_access_denied",
    "invalid_select_user",
    "invalid_select_admin",
];
/// The token endpoint's fixed OAuth error identifiers Mallard reacts to.
pub const TOKEN_ERROR_TAGS: [&str; 2] = ["invalid_grant", "invalid_scope"];

/// The `.tag` of a 401 body, only when it is one of `AUTH_ERROR_TAGS`.
pub fn auth_error_tag(bytes: &[u8]) -> Option<&'static str> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let tag = value["error"][".tag"].as_str()?;
    AUTH_ERROR_TAGS.into_iter().find(|known| *known == tag)
}
/// The `error` of a token endpoint failure, only when it is one of
/// `TOKEN_ERROR_TAGS`.
pub fn token_error_tag(bytes: &[u8]) -> Option<&'static str> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let tag = value["error"].as_str()?;
    TOKEN_ERROR_TAGS.into_iter().find(|known| *known == tag)
}

/// How a rejected request classifies the credential, before the fixed
/// message is chosen for the credential kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthClass {
    /// The credential itself is refused: reconnect or replace it.
    Rejected,
    /// A manual access token ran out; a refresh credential never lands here.
    Expired,
    /// The account or grant lacks a permission.
    Denied,
    /// No usable answer from Dropbox.
    Unreachable,
}

/// The fixed health record for one classified failure.
pub fn health_for(
    class: AuthClass,
    managed: bool,
    evidence: Option<ConnectionEvidence>,
) -> ConnectionHealth {
    let (state, code, message) = match (class, managed) {
        (AuthClass::Rejected, true) => (
            ConnectionState::ReconnectRequired,
            "dropbox_reconnect_required",
            "Dropbox rejected the saved authorization. Reconnect Dropbox to restore access.",
        ),
        (AuthClass::Rejected, false) => (
            ConnectionState::ReconnectRequired,
            "dropbox_reconnect_required",
            "Dropbox rejected the saved access token. Paste a replacement token in Storage settings.",
        ),
        (AuthClass::Expired, _) => (
            ConnectionState::TokenExpired,
            "dropbox_token_expired",
            "The Dropbox access token expired. Paste a replacement token in Storage settings.",
        ),
        (AuthClass::Denied, _) => (
            ConnectionState::PermissionDenied,
            "dropbox_permission_denied",
            "Dropbox denied access for this connection. Reconnect Dropbox to grant the missing permission.",
        ),
        (AuthClass::Unreachable, _) => (
            ConnectionState::Unreachable,
            "dropbox_request_failed",
            "Dropbox could not be reached. Check the network and try again.",
        ),
    };
    ConnectionHealth::failed(state, code, message, evidence)
}

pub enum DropboxCredential {
    AccessToken(SecretValue),
    RefreshToken(SecretValue),
}

pub struct AccessGrant {
    pub token: SecretValue,
    /// Unix seconds on the wall clock. A monotonic clock pauses while the
    /// machine sleeps, which would present a token Dropbox has already
    /// expired; the wall clock keeps counting.
    pub expires_at: u64,
}
pub struct AuthorizationGrant {
    pub access: AccessGrant,
    pub refresh: SecretValue,
    pub account_id: String,
}

#[derive(Clone)]
pub struct DropboxClient {
    client: Client,
    api: String,
    token_url: String,
    content_api: String,
}

impl DropboxClient {
    pub fn new() -> Result<Self, AuthError> {
        Self::with_endpoints(
            "https://api.dropboxapi.com/2",
            "https://api.dropboxapi.com/oauth2/token",
        )
    }

    fn with_endpoints(api: &str, token_url: &str) -> Result<Self, AuthError> {
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|_| AuthError::Local)?,
            api: api.into(),
            token_url: token_url.into(),
            content_api: if api.starts_with("http://127.0.0.1:") {
                api.into()
            } else {
                "https://content.dropboxapi.com/2".into()
            },
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(api: &str, token_url: &str) -> Self {
        for endpoint in [api, token_url] {
            let url = url::Url::parse(endpoint).unwrap();
            assert_eq!(url.scheme(), "http");
            assert_eq!(url.host_str(), Some("127.0.0.1"));
        }
        Self::with_endpoints(api, token_url).unwrap()
    }

    /// Send one request and classify the answer. `endpoint` is the literal
    /// route, kept as evidence; the body is read only for the fixed error
    /// identifier and never retained.
    async fn response(
        &self,
        endpoint: &'static str,
        request: RequestBuilder,
    ) -> Result<Zeroizing<Vec<u8>>, AuthFailure> {
        let evidence = |status: Option<u16>, tag, request_id| {
            Some(ConnectionEvidence {
                endpoint,
                http_status: status,
                error_tag: tag,
                request_id,
            })
        };
        let fail = |error, evidence| AuthFailure { error, evidence };
        let mut response = request
            .send()
            .await
            .map_err(|_| fail(AuthError::Remote, evidence(None, None, None)))?;
        let status = response.status();
        let request_id = ConnectionEvidence::request_id(
            response
                .headers()
                .get("x-dropbox-request-id")
                .map(HeaderValue::as_bytes),
        );
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(fail(
                AuthError::Permission,
                evidence(Some(403), None, request_id),
            ));
        }
        if response
            .content_length()
            .is_some_and(|size| size > RESPONSE_LIMIT as u64)
        {
            return Err(fail(AuthError::Invalid, None));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            fail(
                AuthError::Remote,
                evidence(Some(status.as_u16()), None, None),
            )
        })? {
            if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
                return Err(fail(AuthError::Invalid, None));
            }
            bytes.extend_from_slice(&chunk);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            let tag = auth_error_tag(&bytes);
            let error = match tag {
                Some("missing_scope" | "user_suspended" | "route_access_denied") => {
                    AuthError::Permission
                }
                _ => AuthError::Reconnect,
            };
            return Err(fail(error, evidence(Some(401), tag, request_id)));
        }
        if !status.is_success() {
            let tag = if endpoint == "files/create_folder_v2" && status.as_u16() == 409 {
                folder_conflict_tag(&bytes)
            } else {
                token_error_tag(&bytes)
            };
            let error = match tag {
                Some("invalid_grant") => AuthError::Reconnect,
                Some("invalid_scope") => AuthError::Permission,
                _ => AuthError::Remote,
            };
            return Err(fail(
                error,
                evidence(Some(status.as_u16()), tag, request_id),
            ));
        }
        Ok(bytes)
    }

    async fn token(
        &self,
        fields: &[(&str, &str)],
        scopes: &str,
    ) -> Result<TokenResponse, AuthFailure> {
        let body = Zeroizing::new(
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(fields.iter().copied())
                .finish(),
        );
        let bytes = self
            .response(
                "oauth2/token",
                self.client
                    .post(&self.token_url)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(body.to_string()),
            )
            .await?;
        let plain = |error| AuthFailure {
            error,
            evidence: None,
        };
        let token: TokenResponse =
            serde_json::from_slice(&bytes).map_err(|_| plain(AuthError::Invalid))?;
        if !token.token_type.eq_ignore_ascii_case("bearer")
            || token.expires_in <= 60
            || token.expires_in > 86400
        {
            return Err(plain(AuthError::Invalid));
        }
        if token.scope.as_ref().is_some_and(|actual| {
            scopes
                .split_whitespace()
                .any(|needed| !actual.split_whitespace().any(|s| s == needed))
        }) {
            return Err(AuthFailure {
                error: AuthError::Permission,
                evidence: Some(ConnectionEvidence {
                    endpoint: "oauth2/token",
                    http_status: Some(200),
                    error_tag: Some("invalid_scope"),
                    request_id: None,
                }),
            });
        }
        validate_token(&token.access_token).map_err(plain)?;
        Ok(token)
    }

    pub async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        scopes: &str,
    ) -> Result<AuthorizationGrant, AuthError> {
        let response = self
            .token(
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", APP_KEY),
                    ("redirect_uri", CALLBACK_URI),
                    ("code", code),
                    ("code_verifier", verifier),
                ],
                scopes,
            )
            .await?;
        let refresh = response
            .refresh_token
            .as_deref()
            .ok_or(AuthError::Invalid)?;
        validate_token(refresh)?;
        let account_id = response.account_id.as_deref().ok_or(AuthError::Invalid)?;
        validate_id(account_id)?;
        Ok(AuthorizationGrant {
            access: response.access()?,
            refresh: SecretValue::parse(refresh).map_err(|_| AuthError::Invalid)?,
            account_id: account_id.to_string(),
        })
    }

    /// Exchange the refresh token for a short-lived grant. The failure keeps
    /// the token endpoint's evidence so a rejected refresh can be diagnosed.
    pub async fn refresh(&self, refresh: &SecretValue) -> Result<AccessGrant, AuthFailure> {
        self.token(
            &[
                ("grant_type", "refresh_token"),
                ("client_id", APP_KEY),
                ("refresh_token", refresh.expose()),
                ("scope", STORAGE_SCOPES),
            ],
            STORAGE_SCOPES,
        )
        .await?
        .access()
        .map_err(|error| AuthFailure {
            error,
            evidence: None,
        })
    }

    /// Read only the fixed storage marker. Never create or repair it here.
    pub async fn read_storage_marker(
        &self,
        namespace: &str,
        token: &SecretValue,
    ) -> Result<Option<Vec<u8>>, AuthError> {
        validate_id(namespace)?;
        let mut response = self
            .client
            .post(format!("{}/files/download", self.content_api))
            .header("Authorization", bearer(token)?)
            .header(
                "Dropbox-API-Path-Root",
                serde_json::json!({".tag":"namespace_id", "namespace_id":namespace}).to_string(),
            )
            .header("Dropbox-API-Arg", "{\"path\":\"/.mallard/_storage.json\"}")
            .send()
            .await
            .map_err(|_| AuthError::Remote)?;
        let status = response.status();
        let limit = if status.is_success() { 4096 } else { 16384 };
        if response
            .content_length()
            .is_some_and(|len| len > limit as u64)
        {
            return Err(AuthError::Invalid);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| AuthError::Remote)? {
            if bytes.len().saturating_add(chunk.len()) > limit {
                return Err(AuthError::Invalid);
            }
            bytes.extend_from_slice(&chunk);
        }
        if status.is_success() {
            return Ok(Some(bytes.to_vec()));
        }
        if status.as_u16() == 409 {
            let value: Value = serde_json::from_slice(&bytes).map_err(|_| AuthError::Invalid)?;
            if value["error"][".tag"] == "path" && value["error"]["path"][".tag"] == "not_found" {
                return Ok(None);
            }
        }
        Err(match status.as_u16() {
            401 if !matches!(
                auth_error_tag(&bytes),
                Some("missing_scope" | "user_suspended" | "route_access_denied")
            ) =>
            {
                AuthError::Reconnect
            }
            401 | 403 => AuthError::Permission,
            _ => AuthError::Remote,
        })
    }

    pub async fn rpc(
        &self,
        route: &'static str,
        args: Value,
        token: &SecretValue,
        root: Option<&Value>,
    ) -> Result<Value, AuthError> {
        self.rpc_evidenced(route, args, token, root)
            .await
            .map_err(|failure| failure.error)
    }

    pub(crate) async fn rpc_evidenced(
        &self,
        route: &'static str,
        args: Value,
        token: &SecretValue,
        root: Option<&Value>,
    ) -> Result<Value, AuthFailure> {
        let plain = |error| AuthFailure {
            error,
            evidence: None,
        };
        let mut request = self
            .client
            .post(format!("{}/{route}", self.api))
            .header("Authorization", bearer(token).map_err(plain)?)
            .json(&args);
        if let Some(root) = root {
            request = request.header("Dropbox-API-Path-Root", root.to_string());
        }
        let bytes = self.response(route, request).await?;
        serde_json::from_slice(&bytes).map_err(|_| plain(AuthError::Invalid))
    }
}

fn folder_conflict_tag(bytes: &[u8]) -> Option<&'static str> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let error = &value["error"];
    if error[".tag"] != "path" || error["path"][".tag"] != "conflict" {
        return None;
    }
    match error["path"]["conflict"][".tag"].as_str()? {
        "file" => Some("path/conflict/file"),
        "folder" => Some("path/conflict/folder"),
        _ => None,
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    account_id: Option<String>,
    token_type: String,
    expires_in: u64,
    scope: Option<String>,
}
impl Drop for TokenResponse {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}
impl TokenResponse {
    fn access(&self) -> Result<AccessGrant, AuthError> {
        Ok(AccessGrant {
            token: SecretValue::parse(&self.access_token).map_err(|_| AuthError::Invalid)?,
            expires_at: now_secs().saturating_add(self.expires_in.saturating_sub(30)),
        })
    }
}

pub fn validate_token(value: &str) -> Result<(), AuthError> {
    if value.is_empty() || value.len() > 16 * 1024 || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AuthError::Invalid);
    }
    Ok(())
}
pub fn validate_id(value: &str) -> Result<(), AuthError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
    {
        return Err(AuthError::Invalid);
    }
    Ok(())
}
pub fn bearer(token: &SecretValue) -> Result<HeaderValue, AuthError> {
    let text = Zeroizing::new(format!("Bearer {}", token.expose()));
    let mut header = HeaderValue::from_str(&text).map_err(|_| AuthError::Invalid)?;
    header.set_sensitive(true);
    Ok(header)
}

/// One object store shares this lock across all of its requests. Refresh is
/// performed before dispatch; an uncertain mutation is never replayed. The
/// health cell beside the credential records what Dropbox last said about
/// it; the transport records API answers here too.
pub struct DropboxAuthorization {
    credential: DropboxCredential,
    client: DropboxClient,
    cached: Mutex<Option<AccessGrant>>,
    health: ConnectionHealthCell,
}
impl DropboxAuthorization {
    pub fn new(credential: DropboxCredential, client: DropboxClient) -> Self {
        Self {
            credential,
            client,
            cached: Mutex::new(None),
            health: ConnectionHealthCell::default(),
        }
    }
    /// Whether access renews through a refresh token.
    pub fn is_managed(&self) -> bool {
        matches!(self.credential, DropboxCredential::RefreshToken(_))
    }
    pub fn health(&self) -> Option<ConnectionHealth> {
        self.health.get()
    }
    /// Record a classified failure for the credential this store presents.
    pub fn record(&self, class: AuthClass, evidence: Option<ConnectionEvidence>) {
        self.health
            .record(health_for(class, self.is_managed(), evidence));
    }
    pub fn record_connected(&self) {
        self.health.record_connected();
    }
    /// Drop the cached grant so the next request refreshes first.
    pub async fn invalidate(&self) {
        *self.cached.lock().await = None;
    }
    pub async fn header(&self) -> Result<HeaderValue, AuthError> {
        match &self.credential {
            DropboxCredential::AccessToken(token) => bearer(token),
            DropboxCredential::RefreshToken(refresh) => {
                let mut cached = self.cached.lock().await;
                if cached
                    .as_ref()
                    .is_none_or(|grant| grant.expires_at <= now_secs())
                {
                    *cached = None;
                    let grant = self.client.refresh(refresh).await.map_err(|failure| {
                        let class = match failure.error {
                            AuthError::Reconnect => Some(AuthClass::Rejected),
                            AuthError::Permission => Some(AuthClass::Denied),
                            AuthError::Remote => Some(AuthClass::Unreachable),
                            AuthError::Invalid | AuthError::Local => None,
                        };
                        if let Some(class) = class {
                            self.record(class, failure.evidence);
                        }
                        failure.error
                    })?;
                    *cached = Some(grant);
                }
                bearer(&cached.as_ref().ok_or(AuthError::Invalid)?.token)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dropbox_setup::mock::MockDropboxSetup;

    #[test]
    fn pkce_uses_random_state_s256_offline_and_registered_public_client() {
        let first = AuthorizationRequest::new(SETUP_SCOPES).unwrap();
        let second = AuthorizationRequest::new(SETUP_SCOPES).unwrap();
        let query = first
            .url
            .query_pairs()
            .into_owned()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(first.url.host_str(), Some("www.dropbox.com"));
        assert_eq!(query["redirect_uri"], CALLBACK_URI);
        assert_eq!(query["client_id"], APP_KEY);
        assert_eq!(query["token_access_type"], "offline");
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["code_challenge"].len(), 43);
        assert_eq!(first.verifier.len(), 64);
        assert_ne!(first.state, second.state);
        assert_ne!(first.verifier, second.verifier);
        assert!(!first.url.as_str().contains(first.verifier.as_str()));
        assert!(!query.contains_key("client_secret"));
    }

    #[test]
    fn concurrent_refresh_is_single_flight_and_expired_cache_is_replaced() {
        let mock = MockDropboxSetup::new();
        let auth = DropboxAuthorization::new(
            DropboxCredential::RefreshToken(SecretValue::parse("refresh-canary").unwrap()),
            mock.client(),
        );
        mock.runtime.block_on(async {
            let (a, b, c) = tokio::join!(auth.header(), auth.header(), auth.header());
            for result in [a, b, c] {
                let header = result.unwrap();
                assert_eq!(header, "Bearer test-token");
                assert!(header.is_sensitive());
            }
            assert_eq!(mock.count("refresh"), 1);
            // Expiry is wall-clock seconds, so a grant cached before a sleep
            // is stale on wake. The mock issues four-hour grants.
            let cached = auth.cached.lock().await.as_ref().unwrap().expires_at;
            assert!((cached - now_secs()).abs_diff(14400 - 30) <= 2);
            auth.cached.lock().await.as_mut().unwrap().expires_at = now_secs();
            auth.header().await.unwrap();
            assert_eq!(mock.count("refresh"), 2);
            assert!(auth.health().is_none(), "a refresh alone proves nothing");
            auth.cached.lock().await.as_mut().unwrap().expires_at = 0;
            mock.state.lock().unwrap().deny_refresh = true;
            let (a, b, c) = tokio::join!(auth.header(), auth.header(), auth.header());
            for result in [a, b, c] {
                assert_eq!(result.unwrap_err(), AuthError::Reconnect);
            }
            // One refused refresh under three concurrent callers: the lock
            // serialises them, and each refresh presents the rejected token
            // again only because nothing else could answer.
            assert!(auth.cached.lock().await.is_none());
            let health = auth.health().unwrap();
            assert_eq!(health.state, ConnectionState::ReconnectRequired);
            assert_eq!(health.code, "dropbox_reconnect_required");
            assert!(health.suspends());
            let evidence = health.evidence.clone().unwrap();
            assert_eq!(evidence.endpoint, "oauth2/token");
            assert_eq!(evidence.http_status, Some(400));
            assert_eq!(evidence.error_tag, Some("invalid_grant"));
            assert_eq!(evidence.request_id.as_deref(), Some("req-refresh-1"));
            let described = health.describe();
            assert!(
                described.contains("oauth2/token, HTTP 400, invalid_grant, request req-refresh-1")
            );
            assert!(!described.contains("secret-canary") && !described.contains("refresh-canary"));
        });
    }

    #[test]
    fn manual_token_never_uses_oauth_and_permission_failures_are_redacted() {
        let mock = MockDropboxSetup::new();
        let auth = DropboxAuthorization::new(
            DropboxCredential::AccessToken(SecretValue::parse("manual-canary").unwrap()),
            mock.client(),
        );
        assert_eq!(
            mock.runtime.block_on(auth.header()).unwrap(),
            "Bearer manual-canary"
        );
        assert_eq!(mock.count("oauth2/token"), 0);
        mock.state.lock().unwrap().missing_scope = true;
        assert!(matches!(
            mock.runtime.block_on(mock.client().exchange(
                "secret-code",
                "secret-verifier",
                SETUP_SCOPES
            )),
            Err(AuthError::Permission)
        ));
        mock.state.lock().unwrap().missing_scope = false;
        mock.state.lock().unwrap().redirect_token = true;
        assert!(matches!(
            mock.runtime.block_on(mock.client().exchange(
                "secret-code",
                "secret-verifier",
                SETUP_SCOPES
            )),
            Err(AuthError::Remote)
        ));
        for error in [
            AuthError::Permission,
            AuthError::Remote,
            AuthError::Reconnect,
        ] {
            assert!(!error.message().contains("canary"));
        }
    }
}
