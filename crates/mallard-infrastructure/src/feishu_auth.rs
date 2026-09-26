//! Feishu OAuth v3 with Mallard's own app. The App ID and App Secret ship
//! with the desktop build (`FeishuApp`); the refresh chain lives in the
//! connection document (`feishu_connections`); the access grant lives in
//! memory. No Mallard service takes part: every request travels between
//! this computer and Feishu. Credentials never appear in errors, evidence, or
//! logs: diagnostics keep only fixed identifiers, numeric provider codes,
//! the HTTP status, a short request ID, and PKCE check results and lengths.
use crate::feishu_connections::{
    ConnectionId, FeishuConnection, FeishuConnectionStore, FEISHU_CONNECTION_SCHEMA,
};
use crate::storage_core::{
    ConnectionEvidence, ConnectionHealth, ConnectionHealthCell, ConnectionState,
};
use mallard_domain::{now_secs, SecretValue};
use reqwest::{header::HeaderValue, Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

// Keep authorization and token exchange on v3: the older authorization
// route can return a code that v3 rejects with PKCE error 20049.
pub const AUTHORIZE_URL: &str = "https://accounts.feishu.cn/oauth/v3/authorize";
pub const TOKEN_URL: &str = "https://accounts.feishu.cn/oauth/v3/token";
/// The Open API origin; Drive and `user_info` routes live below it.
pub const API_ORIGIN: &str = "https://open.feishu.cn";
pub const CALLBACK_URI: &str = "http://127.0.0.1:53683/feishu/callback";
pub const CALLBACK_PORT: u16 = 53683;
/// The six scopes a Mallard connection needs, and exactly these: the five
/// the POC approved, plus `offline_access` for the refresh token.
pub const SCOPES: &str = "space:document:retrieve space:folder:create drive:file:upload drive:file:download auth:user.id:read offline_access";
/// Refresh when fewer than this many seconds remain on the access grant.
pub const REFRESH_MARGIN_SECS: u64 = 60;
const RESPONSE_LIMIT: usize = 64 * 1024;
const MAX_NAME_BYTES: usize = 256;

/// The Feishu callback shape for the shared loopback receiver.
pub const FEISHU_CALLBACK: crate::loopback_browser::LoopbackCallback =
    crate::loopback_browser::LoopbackCallback {
        path: "/feishu/callback",
        result_key: "code",
        completion_path: "/feishu/complete",
        title: "Feishu sign-in",
        received_message: "Feishu sign-in received. Return to Mallard and wait for setup to finish.",
        failed_message: "Feishu sign-in was not completed. Return to Mallard.",
        errors: crate::loopback_browser::LoopbackErrors {
            invalid: (
                "feishu_callback_invalid",
                "Feishu sign-in returned an invalid callback. Run setup again.",
            ),
            unavailable: (
                "feishu_callback_unavailable",
                "Mallard could not bind the Feishu callback port. Close the other setup attempt and try again.",
            ),
            browser_failed: (
                "feishu_browser_failed",
                "Mallard could not open the Feishu sign-in page.",
            ),
            cancelled: ("feishu_setup_cancelled", "Feishu setup was cancelled."),
            denied: (
                "feishu_consent_denied",
                "Feishu access was not approved. Setup stopped.",
            ),
            timeout: (
                "feishu_authorization_timeout",
                "Feishu sign-in timed out. Run setup again when you are ready.",
            ),
        },
    };

/// A self-built app's ID: `cli_` and up to 120 letters or digits.
pub fn valid_app_id(value: &str) -> bool {
    value.strip_prefix("cli_").is_some_and(|rest| {
        (1..=120).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// An App Secret as Feishu issues it: printable ASCII, bounded.
pub fn valid_app_secret(value: &str) -> bool {
    (1..=16 * 1024).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_graphic())
}

/// Mallard's own Feishu app: the ID every sign-in link names and the
/// secret Feishu's token endpoint requires for every developer-created app.
/// A build carries both from `MALLARD_FEISHU_APP_ID` and
/// `MALLARD_FEISHU_APP_SECRET` in its environment (`npm run tauri` fills
/// them from the repository's git-ignored `.env.feishu`), and Cargo
/// recompiles this crate when either changes; a development build also
/// reads the same names from its environment at run time. A build with
/// neither still offers Feishu and reports the missing app at sign-in.
///
/// The secret can be read out of a shipped binary, and that is harmless by
/// construction: the app holds no tenant-identity scopes, so the secret
/// alone reaches no data; the redirect is fixed to this computer's loopback
/// port, so a leaked secret cannot complete a sign-in from anywhere else;
/// and a refresh needs the owner-only connection document, which only local
/// access reads.
#[derive(Clone)]
pub struct FeishuApp {
    pub app_id: String,
    pub app_secret: SecretValue,
}

impl FeishuApp {
    pub fn new(app_id: impl Into<String>, app_secret: SecretValue) -> Result<Self, AuthError> {
        let app_id = app_id.into();
        if !valid_app_id(&app_id) || !valid_app_secret(app_secret.expose()) {
            return Err(AuthError::Invalid);
        }
        Ok(Self { app_id, app_secret })
    }

    /// The app this build carries, or nothing.
    pub fn built_in() -> Option<Self> {
        let baked = (
            option_env!("MALLARD_FEISHU_APP_ID").map(str::to_owned),
            option_env!("MALLARD_FEISHU_APP_SECRET").map(str::to_owned),
        );
        let (app_id, app_secret) = if cfg!(debug_assertions) {
            (
                std::env::var("MALLARD_FEISHU_APP_ID").ok().or(baked.0),
                std::env::var("MALLARD_FEISHU_APP_SECRET").ok().or(baked.1),
            )
        } else {
            baked
        };
        Self::new(app_id?, SecretValue::parse(app_secret?).ok()?).ok()
    }
}

pub struct AuthorizationRequest {
    pub url: url::Url,
    pub state: Zeroizing<String>,
    pub verifier: Zeroizing<String>,
}

/// Check results for one attempt. Never retain the URL, challenge, state,
/// verifier, callback code, or token in diagnostic output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PkceDiagnostics {
    pub challenge_present: bool,
    pub method_s256: bool,
    pub verifier_matches_challenge: bool,
    pub callback_state_verified: bool,
    pub verifier_length: usize,
    pub challenge_length: usize,
}

impl AuthorizationRequest {
    /// Call only after the loopback receiver accepts this request's state,
    /// immediately before exchanging its code with this retained verifier.
    pub fn diagnostics_after_verified_callback(&self) -> PkceDiagnostics {
        use sha2::{Digest, Sha256};
        let mut challenges = self
            .url
            .query_pairs()
            .filter(|(k, _)| k == "code_challenge");
        let challenge = challenges.next().map(|(_, value)| value);
        let challenge_present = challenge.as_ref().is_some_and(|value| !value.is_empty())
            && challenges.next().is_none();
        let mut methods = self
            .url
            .query_pairs()
            .filter(|(k, _)| k == "code_challenge_method");
        let method_s256 =
            methods.next().is_some_and(|(_, value)| value == "S256") && methods.next().is_none();
        let expected = aws_smithy_types::base64::encode(Sha256::digest(self.verifier.as_bytes()))
            .trim_end_matches('=')
            .replace('+', "-")
            .replace('/', "_");
        PkceDiagnostics {
            challenge_present,
            method_s256,
            verifier_matches_challenge: challenge_present
                && challenge.as_deref() == Some(expected.as_str()),
            callback_state_verified: true,
            verifier_length: self.verifier.len(),
            challenge_length: challenge.as_ref().map_or(0, |value| value.len()),
        }
    }

    pub fn new(app_id: &str) -> Result<Self, AuthError> {
        use sha2::{Digest, Sha256};
        if !valid_app_id(app_id) {
            return Err(AuthError::Invalid);
        }
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
        let mut url = url::Url::parse(AUTHORIZE_URL).map_err(|_| AuthError::Local)?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", app_id),
            ("response_type", "code"),
            ("redirect_uri", CALLBACK_URI),
            ("scope", SCOPES),
            ("state", state.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
        ]);
        Ok(Self {
            url,
            state,
            verifier,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthError {
    /// The saved sign-in is refused: revoked, expired, or its refresh token
    /// was already used. Only a new sign-in recovers.
    Reconnect,
    /// Feishu refused the App ID or App Secret.
    AppRejected,
    /// The grant lacks a permission, or consent returned other scopes.
    Permission,
    /// The exchange returned a scope set other than the six.
    ScopeMismatch,
    Invalid,
    /// No usable answer from Feishu.
    Remote,
    Local,
    /// The connection document could not be read or written.
    NotSaved,
}

impl AuthError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Reconnect => "feishu_reconnect_required",
            Self::AppRejected => "feishu_app_rejected",
            Self::Permission => "feishu_permission_denied",
            Self::ScopeMismatch => "feishu_scope_mismatch",
            Self::Invalid => "feishu_response_invalid",
            Self::Remote => "feishu_request_failed",
            Self::Local => "feishu_client_failed",
            Self::NotSaved => "feishu_connection_not_saved",
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::Reconnect => "Feishu rejected the saved sign-in. Reconnect Feishu in Storage settings.",
            Self::AppRejected => "Feishu refused Mallard's app. Update Mallard, then try again.",
            Self::Permission => "Feishu denied access. Check the app's permissions and the user's consent.",
            Self::ScopeMismatch => "Feishu granted different permissions than Mallard needs. Check the app's scopes with your Feishu admin.",
            Self::Invalid => "Feishu returned an invalid sign-in or setup response.",
            Self::Remote => "The Feishu request did not complete. No write was retried.",
            Self::Local => "Mallard could not start the Feishu connection.",
            Self::NotSaved => "Mallard could not save the renewed Feishu connection.",
        }
    }
}

/// A failure with the evidence its response carried, and whether Feishu
/// may have acted on the request before the answer was lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthFailure {
    pub error: AuthError,
    pub evidence: Option<ConnectionEvidence>,
    /// Numeric provider error only; never retain response text or credentials.
    pub provider_code: Option<u64>,
    /// True when the request may have reached Feishu and its answer was
    /// lost: a timeout, a dropped body, a server error. False when the
    /// answer is definitive, or the request never left this computer.
    pub outcome_unknown: bool,
}

impl From<AuthFailure> for AuthError {
    fn from(failure: AuthFailure) -> Self {
        failure.error
    }
}

/// The token endpoint's fixed OAuth error identifiers. Nothing else from
/// a body is ever retained.
pub const TOKEN_ERROR_TAGS: [&str; 9] = [
    "invalid_request",
    "invalid_client",
    "invalid_grant",
    "unauthorized_client",
    "unsupported_grant_type",
    "invalid_scope",
    "access_denied",
    "server_error",
    "temporarily_unavailable",
];
/// The one numeric token-endpoint code Mallard reacts to by name: the
/// refresh token was already used, so an earlier refresh landed.
pub const REFRESH_TOKEN_USED_CODE: u64 = 20073;
pub const REFRESH_TOKEN_USED_TAG: &str = "20073";

fn token_error(bytes: &[u8]) -> (Option<&'static str>, Option<u64>) {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return (None, None);
    };
    let tag = value["error"]
        .as_str()
        .and_then(|tag| TOKEN_ERROR_TAGS.into_iter().find(|known| *known == tag));
    (tag, value["code"].as_u64())
}

/// How a rejected request classifies the connection, before the fixed
/// message is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthClass {
    /// The saved sign-in is refused: reconnect.
    Rejected,
    /// Mallard's App ID or App Secret is refused: a newer Mallard fixes it.
    AppRejected,
    /// Feishu's 365-day limit ran out: sign in again.
    Reauthorize,
    /// This computer holds no document for the connection.
    Missing,
    /// The account or grant lacks a permission.
    Denied,
    /// No usable answer from Feishu.
    Unreachable,
    /// A request completed with an answer Mallard could not use.
    RequestFailed,
    /// The connection document could not be read or written.
    Document,
}

pub fn health_for(class: AuthClass, evidence: Option<ConnectionEvidence>) -> ConnectionHealth {
    let (state, code, message) = match class {
        AuthClass::Rejected => (
            ConnectionState::ReconnectRequired,
            "feishu_reconnect_required",
            "Feishu rejected the saved sign-in. Reconnect Feishu in Storage settings.",
        ),
        // Mallard's app, not the user's sign-in: nothing here recovers it,
        // and a reconnect would fail the same way. A newer Mallard does.
        AuthClass::AppRejected => (
            ConnectionState::Unreachable,
            "feishu_app_rejected",
            "Feishu refused Mallard's app. Update Mallard, then try again.",
        ),
        AuthClass::Reauthorize => (
            ConnectionState::ReconnectRequired,
            "feishu_reconnect_required",
            "Feishu requires a fresh sign-in for this connection. Reconnect Feishu in Storage settings.",
        ),
        AuthClass::Missing => (
            ConnectionState::ReconnectRequired,
            "feishu_reconnect_required",
            "This computer holds no Feishu sign-in for this storage. Reconnect Feishu in Storage settings.",
        ),
        AuthClass::Denied => (
            ConnectionState::PermissionDenied,
            "feishu_permission_denied",
            "Feishu denied access for this connection. Reconnect Feishu to grant the missing permission.",
        ),
        AuthClass::Unreachable => (
            ConnectionState::Unreachable,
            "feishu_unreachable",
            "Feishu could not be reached. Check the network and try again.",
        ),
        AuthClass::RequestFailed => (
            ConnectionState::Unreachable,
            "feishu_request_failed",
            "The Feishu request did not complete. No write was retried.",
        ),
        AuthClass::Document => (
            ConnectionState::Unreachable,
            "feishu_connection_not_saved",
            "Mallard could not save the renewed Feishu connection.",
        ),
    };
    ConnectionHealth::failed(state, code, message, evidence)
}

#[derive(Debug)]
pub struct AccessGrant {
    pub token: SecretValue,
    /// Unix seconds on the wall clock, so a grant cached before a sleep is
    /// stale on wake.
    pub expires_at: u64,
}

/// What a refresh returns: a new access grant and the rotated refresh
/// token with the time Feishu says it runs out.
#[derive(Debug)]
pub struct RefreshedGrant {
    pub access: AccessGrant,
    pub refresh: SecretValue,
    pub refresh_expires_at: u64,
}

/// What a sign-in returns. `reauthorize_at` is Feishu's 365-day limit,
/// counted from this consent.
#[derive(Debug)]
pub struct AuthorizationGrant {
    pub access: AccessGrant,
    pub refresh: SecretValue,
    pub reauthorize_at: u64,
}

impl AuthorizationGrant {
    /// The document a sign-in writes under `connection_id`.
    pub fn connection(&self, connection_id: ConnectionId) -> FeishuConnection {
        FeishuConnection {
            schema: FEISHU_CONNECTION_SCHEMA,
            connection_id,
            refresh_token: self.refresh.clone(),
            reauthorize_at: self.reauthorize_at,
            refresh_uncertain: false,
            updated_at: now_secs().max(1),
        }
    }
}

/// The signed-in user, as `user_info` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub name: String,
}

#[derive(Clone)]
pub struct FeishuClient {
    client: Client,
    token_url: String,
    api: String,
}

impl FeishuClient {
    pub fn new() -> Result<Self, AuthError> {
        Self::with_endpoints(API_ORIGIN, TOKEN_URL)
    }

    fn with_endpoints(api: &str, token_url: &str) -> Result<Self, AuthError> {
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| AuthError::Local)?,
            token_url: token_url.into(),
            api: api.into(),
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

    /// The Open API origin this client talks to.
    pub fn api(&self) -> &str {
        &self.api
    }

    /// Send one request and classify the answer. The body is read only for
    /// the fixed error identifier and never retained.
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
        let fail = |error, evidence, outcome_unknown| AuthFailure {
            provider_code: None,
            error,
            evidence,
            outcome_unknown,
        };
        let mut response = request.send().await.map_err(|error| {
            // A connection that never opened carried nothing to Feishu; a
            // timeout after that may have.
            fail(
                AuthError::Remote,
                evidence(None, None, None),
                !error.is_connect(),
            )
        })?;
        let status = response.status();
        let request_id = ConnectionEvidence::request_id(
            response
                .headers()
                .get("x-tt-logid")
                .map(HeaderValue::as_bytes),
        );
        if response
            .content_length()
            .is_some_and(|size| size > RESPONSE_LIMIT as u64)
        {
            return Err(fail(AuthError::Invalid, None, true));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            fail(
                AuthError::Remote,
                evidence(Some(status.as_u16()), None, None),
                true,
            )
        })? {
            if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
                return Err(fail(AuthError::Invalid, None, true));
            }
            bytes.extend_from_slice(&chunk);
        }
        let (tag, code) = token_error(&bytes);
        let fail = |error, evidence, outcome_unknown| AuthFailure {
            provider_code: code.filter(|code| *code != 0),
            error,
            evidence,
            outcome_unknown,
        };
        if status.is_server_error() {
            return Err(fail(
                AuthError::Remote,
                evidence(Some(status.as_u16()), tag, request_id),
                true,
            ));
        }
        if status.as_u16() == 429 {
            return Err(fail(
                AuthError::Remote,
                evidence(Some(429), None, request_id),
                false,
            ));
        }
        if status.is_success() {
            // Feishu answers business failures inside a 200 envelope too.
            if code.is_some_and(|code| code != 0) {
                return Err(fail(
                    classify_token_failure(tag, code),
                    evidence(Some(status.as_u16()), evidence_tag(tag, code), request_id),
                    false,
                ));
            }
            return Ok(bytes);
        }
        Err(fail(
            classify_token_failure(tag, code),
            evidence(Some(status.as_u16()), evidence_tag(tag, code), request_id),
            false,
        ))
    }

    async fn token(&self, fields: &[(&str, &str)]) -> Result<TokenResponse, AuthFailure> {
        let body = Zeroizing::new(
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(fields.iter().copied())
                .finish(),
        );
        let bytes = self
            .response(
                "oauth/v3/token",
                self.client
                    .post(&self.token_url)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(body.to_string()),
            )
            .await?;
        let plain = |error| AuthFailure {
            provider_code: None,
            error,
            evidence: None,
            outcome_unknown: false,
        };
        let token: TokenResponse =
            serde_json::from_slice(&bytes).map_err(|_| plain(AuthError::Invalid))?;
        if token.code != 0
            || !token.token_type.eq_ignore_ascii_case("bearer")
            || token.expires_in < 30
            || token.expires_in > 86_400
        {
            return Err(plain(AuthError::Invalid));
        }
        validate_token(&token.access_token).map_err(plain)?;
        Ok(token)
    }

    /// Exchange the callback code. The returned scope set must be exactly
    /// the six; a refresh token proves `offline_access` when Feishu does
    /// not echo it.
    pub async fn exchange(
        &self,
        app: &FeishuApp,
        code: &str,
        verifier: &str,
    ) -> Result<AuthorizationGrant, AuthFailure> {
        let plain = |error| AuthFailure {
            provider_code: None,
            error,
            evidence: None,
            outcome_unknown: false,
        };
        let response = self
            .token(&[
                ("grant_type", "authorization_code"),
                ("client_id", &app.app_id),
                ("client_secret", app.app_secret.expose()),
                ("code", code),
                ("redirect_uri", CALLBACK_URI),
                ("code_verifier", verifier),
                ("scope", SCOPES),
            ])
            .await?;
        let refresh = response.refresh_token.as_deref().ok_or(AuthFailure {
            provider_code: None,
            error: AuthError::ScopeMismatch,
            evidence: Some(ConnectionEvidence {
                endpoint: "oauth/v3/token",
                http_status: Some(200),
                error_tag: Some("invalid_scope"),
                request_id: None,
            }),
            outcome_unknown: false,
        })?;
        validate_token(refresh).map_err(plain)?;
        let mut granted: std::collections::BTreeSet<&str> = response
            .scope
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        granted.insert("offline_access");
        let needed: std::collections::BTreeSet<&str> = SCOPES.split_whitespace().collect();
        if granted != needed {
            return Err(AuthFailure {
                provider_code: None,
                error: AuthError::ScopeMismatch,
                evidence: Some(ConnectionEvidence {
                    endpoint: "oauth/v3/token",
                    http_status: Some(200),
                    error_tag: Some("invalid_scope"),
                    request_id: None,
                }),
                outcome_unknown: false,
            });
        }
        let reauthorize_at = response.refresh_expiry().map_err(plain)?;
        Ok(AuthorizationGrant {
            access: response.access().map_err(plain)?,
            refresh: SecretValue::parse(refresh).map_err(|_| plain(AuthError::Invalid))?,
            reauthorize_at,
        })
    }

    /// Present the refresh token once. A rotated token comes back with
    /// every success; the failure keeps the token endpoint's evidence.
    pub async fn refresh(
        &self,
        app: &FeishuApp,
        refresh: &SecretValue,
    ) -> Result<RefreshedGrant, AuthFailure> {
        let plain = |error| AuthFailure {
            provider_code: None,
            error,
            evidence: None,
            outcome_unknown: false,
        };
        let response = self
            .token(&[
                ("grant_type", "refresh_token"),
                ("client_id", &app.app_id),
                ("client_secret", app.app_secret.expose()),
                ("refresh_token", refresh.expose()),
            ])
            .await?;
        let rotated = response
            .refresh_token
            .as_deref()
            .ok_or_else(|| plain(AuthError::Invalid))?;
        validate_token(rotated).map_err(plain)?;
        Ok(RefreshedGrant {
            access: response.access().map_err(plain)?,
            refresh: SecretValue::parse(rotated).map_err(|_| plain(AuthError::Invalid))?,
            refresh_expires_at: response.refresh_expiry().map_err(plain)?,
        })
    }

    /// The signed-in user's name and open ID, for the Account line.
    pub async fn user_info(&self, token: &SecretValue) -> Result<Account, AuthFailure> {
        let plain = |error| AuthFailure {
            provider_code: None,
            error,
            evidence: None,
            outcome_unknown: false,
        };
        let bytes = self
            .response(
                "authen/v1/user_info",
                self.client
                    .get(format!("{}/open-apis/authen/v1/user_info", self.api))
                    .header("Authorization", bearer(token).map_err(plain)?),
            )
            .await?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| plain(AuthError::Invalid))?;
        if value["code"] != 0 {
            return Err(plain(AuthError::Invalid));
        }
        let id = value["data"]["open_id"]
            .as_str()
            .filter(|id| validate_id(id).is_ok())
            .ok_or_else(|| plain(AuthError::Invalid))?;
        let name = value["data"]["name"]
            .as_str()
            .map(str::trim)
            .filter(|name| {
                !name.is_empty()
                    && name.len() <= MAX_NAME_BYTES
                    && !name.chars().any(char::is_control)
            })
            .ok_or_else(|| plain(AuthError::Invalid))?;
        Ok(Account {
            id: id.to_string(),
            name: name.to_string(),
        })
    }
}

impl FeishuClient {
    /// One Drive request with the signed-in token; the envelope's `data`.
    async fn drive(
        &self,
        endpoint: &'static str,
        token: &SecretValue,
        request: RequestBuilder,
    ) -> Result<Value, AuthFailure> {
        let plain = |error| AuthFailure {
            provider_code: None,
            error,
            evidence: None,
            outcome_unknown: false,
        };
        let bytes = self
            .response(
                endpoint,
                request.header("Authorization", bearer(token).map_err(plain)?),
            )
            .await?;
        let mut value: Value =
            serde_json::from_slice(&bytes).map_err(|_| plain(AuthError::Invalid))?;
        if value["code"] != 0 {
            return Err(plain(AuthError::Invalid));
        }
        value
            .get_mut("data")
            .map(Value::take)
            .filter(Value::is_object)
            .ok_or_else(|| plain(AuthError::Invalid))
    }

    /// Create `name` at the top of the user's My Space; the new folder's
    /// token. Feishu allows two folders of one name, so nothing is checked
    /// first and nothing is retried.
    pub async fn create_root_folder(
        &self,
        token: &SecretValue,
        name: &str,
    ) -> Result<String, AuthFailure> {
        let plain = |error| AuthFailure {
            provider_code: None,
            error,
            evidence: None,
            outcome_unknown: false,
        };
        let data = self
            .drive(
                "drive/v1/files/create_folder",
                token,
                self.client
                    .post(format!(
                        "{}/open-apis/drive/v1/files/create_folder",
                        self.api
                    ))
                    .json(&serde_json::json!({"name": name, "folder_token": ""})),
            )
            .await?;
        let created = data["token"]
            .as_str()
            .ok_or_else(|| plain(AuthError::Invalid))?;
        validate_id(created).map_err(plain)?;
        Ok(created.to_owned())
    }

    /// One listing of `folder`, page size one: the folder is reachable
    /// with this sign-in.
    pub async fn folder_reachable(
        &self,
        token: &SecretValue,
        folder: &str,
    ) -> Result<(), AuthFailure> {
        let plain = |error| AuthFailure {
            provider_code: None,
            error,
            evidence: None,
            outcome_unknown: false,
        };
        validate_id(folder).map_err(plain)?;
        let url = url::Url::parse_with_params(
            &format!("{}/open-apis/drive/v1/files", self.api),
            [
                ("folder_token", folder),
                ("page_size", "1"),
                ("order_by", "CreatedTime"),
                ("direction", "ASC"),
            ],
        )
        .map_err(|_| plain(AuthError::Invalid))?;
        let data = self
            .drive("drive/v1/files", token, self.client.get(url))
            .await?;
        if !data["files"].is_array() || !data["has_more"].is_boolean() {
            return Err(plain(AuthError::Invalid));
        }
        Ok(())
    }
}

fn classify_token_failure(tag: Option<&'static str>, code: Option<u64>) -> AuthError {
    if code == Some(REFRESH_TOKEN_USED_CODE) {
        return AuthError::Reconnect;
    }
    match tag {
        Some("invalid_grant") => AuthError::Reconnect,
        Some("invalid_client" | "unauthorized_client") => AuthError::AppRejected,
        Some("invalid_scope" | "access_denied") => AuthError::Permission,
        Some("server_error" | "temporarily_unavailable") => AuthError::Remote,
        _ => AuthError::Invalid,
    }
}

/// The evidence tag: the used-refresh-token code by name, else the fixed
/// OAuth error identifier.
fn evidence_tag(tag: Option<&'static str>, code: Option<u64>) -> Option<&'static str> {
    if code == Some(REFRESH_TOKEN_USED_CODE) {
        Some(REFRESH_TOKEN_USED_TAG)
    } else {
        tag
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    code: u64,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    token_type: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token_expires_in: Option<u64>,
    #[serde(default)]
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
            expires_at: now_secs().saturating_add(self.expires_in),
        })
    }
    /// When the returned refresh token runs out: at most 400 days from now.
    fn refresh_expiry(&self) -> Result<u64, AuthError> {
        let seconds = self.refresh_token_expires_in.ok_or(AuthError::Invalid)?;
        if seconds == 0 || seconds > 400 * 86_400 {
            return Err(AuthError::Invalid);
        }
        Ok(now_secs().saturating_add(seconds))
    }
}

pub fn validate_token(value: &str) -> Result<(), AuthError> {
    if value.is_empty() || value.len() > 16 * 1024 || !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AuthError::Invalid);
    }
    Ok(())
}

/// A Feishu token or open ID: letters, digits, `-` and `_`, bounded.
pub fn validate_id(value: &str) -> Result<(), AuthError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(AuthError::Invalid);
    }
    Ok(())
}

/// The folder token in a pasted Drive link, or the pasted token itself.
pub fn folder_token_from_link(value: &str) -> Option<String> {
    let value = value.trim();
    let token = if let Ok(url) = url::Url::parse(value) {
        if !url
            .host_str()
            .is_some_and(|host| host.ends_with(".feishu.cn") || host.ends_with(".larksuite.com"))
        {
            return None;
        }
        url.path().rsplit('/').next().map(str::to_owned)?
    } else {
        value.to_owned()
    };
    let valid = !token.is_empty()
        && token.len() <= 256
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    valid.then_some(token)
}

pub fn bearer(token: &SecretValue) -> Result<HeaderValue, AuthError> {
    let text = Zeroizing::new(format!("Bearer {}", token.expose()));
    let mut header = HeaderValue::from_str(&text).map_err(|_| AuthError::Invalid)?;
    header.set_sensitive(true);
    Ok(header)
}

/// One object store shares this across all of its requests: the access
/// grant in memory, the refresh chain in the connection document, one
/// refresh in flight at a time. Every refresh attempt sends one request.
pub struct FeishuAuthorization {
    app: FeishuApp,
    connection_id: ConnectionId,
    store: FeishuConnectionStore,
    client: FeishuClient,
    cached: Mutex<Option<AccessGrant>>,
    health: ConnectionHealthCell,
}

impl FeishuAuthorization {
    pub fn new(
        app: FeishuApp,
        connection_id: ConnectionId,
        store: FeishuConnectionStore,
        client: FeishuClient,
    ) -> Self {
        Self {
            app,
            connection_id,
            store,
            client,
            cached: Mutex::new(None),
            health: ConnectionHealthCell::default(),
        }
    }

    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    pub fn health(&self) -> Option<ConnectionHealth> {
        self.health.get()
    }

    pub fn record(&self, class: AuthClass, evidence: Option<ConnectionEvidence>) {
        self.health.record(health_for(class, evidence));
    }

    pub fn record_connected(&self) {
        self.health.record_connected();
    }

    /// Drop the cached grant so the next request refreshes first.
    pub async fn invalidate(&self) {
        *self.cached.lock().await = None;
    }

    pub async fn header(&self) -> Result<HeaderValue, AuthError> {
        let mut cached = self.cached.lock().await;
        let fresh = cached
            .as_ref()
            .is_some_and(|grant| grant.expires_at > now_secs().saturating_add(REFRESH_MARGIN_SECS));
        if !fresh {
            *cached = None;
            self.refresh_into(&mut cached).await?;
        }
        bearer(&cached.as_ref().ok_or(AuthError::Invalid)?.token)
    }

    /// One refresh attempt, under the caller's lock: the document is
    /// marked uncertain before the request and settled after a definitive
    /// answer. A grant that arrived but could not be saved is still cached,
    /// so the session continues and the next launch reconnects.
    async fn refresh_into(&self, cached: &mut Option<AccessGrant>) -> Result<(), AuthError> {
        let now = now_secs();
        let mut document = match self.store.load(&self.connection_id) {
            Ok(Some(document)) => document,
            Ok(None) => {
                self.record(AuthClass::Missing, None);
                return Err(AuthError::Reconnect);
            }
            Err(_) => {
                self.record(AuthClass::Document, None);
                return Err(AuthError::NotSaved);
            }
        };
        if now >= document.reauthorize_at {
            self.record(AuthClass::Reauthorize, None);
            return Err(AuthError::Reconnect);
        }
        document.refresh_uncertain = true;
        document.updated_at = now.max(document.updated_at);
        if self.store.save(&document).is_err() {
            self.record(AuthClass::Document, None);
            return Err(AuthError::NotSaved);
        }
        match self
            .client
            .refresh(&self.app, &document.refresh_token)
            .await
        {
            Ok(refreshed) => {
                document.refresh_token = refreshed.refresh;
                document.refresh_uncertain = false;
                // Never extended: the limit counts from the user's consent.
                document.reauthorize_at = document.reauthorize_at.min(refreshed.refresh_expires_at);
                document.updated_at = now_secs().max(document.updated_at);
                *cached = Some(refreshed.access);
                if self.store.save(&document).is_err() {
                    self.record(AuthClass::Document, None);
                    return Err(AuthError::NotSaved);
                }
                Ok(())
            }
            Err(failure) => {
                let class = match failure.error {
                    AuthError::Reconnect => AuthClass::Rejected,
                    AuthError::AppRejected => AuthClass::AppRejected,
                    AuthError::Permission | AuthError::ScopeMismatch => AuthClass::Denied,
                    AuthError::Remote => AuthClass::Unreachable,
                    AuthError::Invalid | AuthError::Local | AuthError::NotSaved => {
                        AuthClass::RequestFailed
                    }
                };
                if !failure.outcome_unknown {
                    // A definitive answer: the stored token was not consumed.
                    document.refresh_uncertain = false;
                    let _ = self.store.save(&document);
                }
                self.record(class, failure.evidence);
                Err(failure.error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage_core::feishu_store::mock::MockFeishuServer;

    fn authorization(server: &MockFeishuServer, app: FeishuApp) -> FeishuAuthorization {
        FeishuAuthorization::new(
            app,
            server.connection_id.clone(),
            server.connections.clone(),
            server.client(),
        )
    }

    #[test]
    fn authorize_link_carries_pkce_state_callback_and_exactly_six_scopes() {
        let first = AuthorizationRequest::new("cli_test").unwrap();
        let second = AuthorizationRequest::new("cli_test").unwrap();
        let query = first
            .url
            .query_pairs()
            .into_owned()
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(first.url.host_str(), Some("accounts.feishu.cn"));
        assert_eq!(first.url.path(), "/oauth/v3/authorize");
        assert_eq!(query["redirect_uri"], CALLBACK_URI);
        assert_eq!(query["client_id"], "cli_test");
        assert_eq!(query["scope"], SCOPES);
        assert_eq!(query["scope"].split_whitespace().count(), 6);
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(query["code_challenge"].len(), 43);
        assert_ne!(first.state, second.state);
        assert_ne!(first.verifier, second.verifier);
        assert!(!first.url.as_str().contains(first.verifier.as_str()));
        assert!(!query.contains_key("client_secret"));
        assert!(AuthorizationRequest::new("not-an-app").is_err());
        assert!(valid_app_id("cli_a1b2c3"));
        assert!(!valid_app_id("cli_") && !valid_app_id("cli_with space") && !valid_app_id("app_x"));
        assert!(
            valid_app_secret("s3cret") && !valid_app_secret("") && !valid_app_secret("has space")
        );
    }

    #[test]
    fn pkce_diagnostics_check_retained_values_and_serialize_only_results() {
        let mut request = AuthorizationRequest::new("cli_test").unwrap();
        // RFC 7636 Appendix B: fixed values independent of our URL builder.
        *request.verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into();
        request.url.set_query(Some(
            "code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&code_challenge_method=S256",
        ));
        let diagnostics = request.diagnostics_after_verified_callback();
        assert_eq!(
            serde_json::to_value(diagnostics).unwrap(),
            serde_json::json!({
                "challenge_present": true,
                "method_s256": true,
                "verifier_matches_challenge": true,
                "callback_state_verified": true,
                "verifier_length": 43,
                "challenge_length": 43
            })
        );
        request.verifier.push('x');
        let changed = request.diagnostics_after_verified_callback();
        assert!(!changed.verifier_matches_challenge);
        assert_eq!(changed.verifier_length, 44);
        for query in [None, Some("code_challenge=&code_challenge_method=plain"),
            Some("code_challenge=first&code_challenge=second&code_challenge_method=S256&code_challenge_method=S256")] {
            request.url.set_query(query);
            let invalid = request.diagnostics_after_verified_callback();
            assert!(!invalid.challenge_present);
            assert!(!invalid.method_s256);
            assert!(!invalid.verifier_matches_challenge);
        }
        let other = AuthorizationRequest::new("cli_test").unwrap();
        request.url = other.url;
        assert!(
            !request
                .diagnostics_after_verified_callback()
                .verifier_matches_challenge
        );
    }

    #[test]
    fn diagnostic_parser_accepts_only_numeric_codes_and_known_error_tags() {
        assert_eq!(
            token_error(
                br#"{"code":20049,"error":"invalid_grant","error_description":"secret-canary"}"#
            ),
            (Some("invalid_grant"), Some(20049))
        );
        assert_eq!(
            token_error(br#"{"code":"secret-canary","error":"token-canary"}"#),
            (None, None)
        );
    }

    #[test]
    fn folder_links_yield_their_token_and_nothing_else_does() {
        assert_eq!(
            folder_token_from_link("https://example.feishu.cn/drive/folder/fldcnAbC123?from=share")
                .as_deref(),
            Some("fldcnAbC123")
        );
        assert_eq!(
            folder_token_from_link(" fldcnAbC123 ").as_deref(),
            Some("fldcnAbC123")
        );
        assert_eq!(
            folder_token_from_link("https://evil.test/drive/folder/fldcnAbC123"),
            None
        );
        assert_eq!(folder_token_from_link(""), None);
        assert_eq!(folder_token_from_link("has space"), None);
        assert_eq!(
            folder_token_from_link("https://example.feishu.cn/drive/folder/"),
            None
        );
    }
    #[test]
    fn exchange_refuses_other_scope_sets_and_wrong_app_credentials() {
        let server = MockFeishuServer::new();
        let client = server.client();
        let app = MockFeishuServer::app();
        let grant = server
            .runtime()
            .block_on(client.exchange(&app, "callback-code", "verifier"))
            .unwrap();
        assert!(grant.reauthorize_at > now_secs() + 300 * 86_400);
        assert!(grant.access.expires_at > now_secs() + 7000);
        assert_eq!(server.count("exchange"), 1);
        server.state.lock().unwrap().scope_override = Some("space:document:retrieve".into());
        let failure = server
            .runtime()
            .block_on(client.exchange(&app, "callback-code", "verifier"))
            .unwrap_err();
        assert_eq!(failure.error, AuthError::ScopeMismatch);
        server.state.lock().unwrap().scope_override = None;
        let wrong =
            FeishuApp::new("cli_test", SecretValue::parse("wrong-secret").unwrap()).unwrap();
        let failure = server
            .runtime()
            .block_on(client.exchange(&wrong, "callback-code", "verifier"))
            .unwrap_err();
        assert_eq!(failure.error, AuthError::AppRejected);
        assert_eq!(failure.provider_code, Some(20011));
        let diagnostic = format!("{failure:?}");
        for secret in [
            "wrong-secret",
            "secret-canary-description",
            "callback-code",
            "verifier",
        ] {
            assert!(!diagnostic.contains(secret));
        }
        let evidence = failure.evidence.unwrap();
        assert_eq!(evidence.http_status, Some(400));
        assert_eq!(evidence.request_id.as_deref(), Some("req-token-1"));
        assert_eq!(evidence.endpoint, "oauth/v3/token");
        assert_eq!(evidence.error_tag, Some("invalid_client"));
        assert!(!failure.outcome_unknown);
        for error in [
            AuthError::AppRejected,
            AuthError::Reconnect,
            AuthError::Remote,
        ] {
            assert!(!error.message().contains("secret"));
        }
        assert!(FeishuApp::new("app_x", SecretValue::parse("s").unwrap()).is_err());
        assert!(FeishuApp::new("cli_ok", SecretValue::parse("has space").unwrap()).is_err());
    }

    #[test]
    fn refresh_rotates_under_one_lock_writes_the_document_twice_and_caches_by_expiry() {
        let server = MockFeishuServer::new();
        let auth = authorization(&server, MockFeishuServer::app());
        let root = server.connections.clone();
        server.runtime().block_on(async {
            let (a, b, c) = tokio::join!(auth.header(), auth.header(), auth.header());
            for result in [a, b, c] {
                let header = result.unwrap();
                assert!(header.is_sensitive());
                assert!(header.to_str().unwrap().starts_with("Bearer access-"));
            }
            assert_eq!(server.count("refresh"), 1);
            // Written uncertain before the request, settled after it.
            assert_eq!(server.uncertain_at_refreshes(), vec![true]);
            let document = root.load(&server.connection_id).unwrap().unwrap();
            assert!(!document.refresh_uncertain);
            assert_ne!(
                document.refresh_token.expose(),
                MockFeishuServer::FIRST_REFRESH
            );
            // `expires_in` is honored: 600 s caches for 600 s, no fixed schedule.
            server.state.lock().unwrap().expires_in = 600;
            auth.cached.lock().await.as_mut().unwrap().expires_at = now_secs() + 30;
            auth.header().await.unwrap();
            assert_eq!(server.count("refresh"), 2);
            let cached = auth.cached.lock().await.as_ref().unwrap().expires_at;
            assert!((cached - now_secs()).abs_diff(600) <= 2);
            auth.header().await.unwrap();
            assert_eq!(server.count("refresh"), 2, "a fresh grant is reused");
            assert!(auth.health().is_none(), "a refresh alone proves nothing");
        });
        // The rotation touched the document only.
        let names: Vec<_> = std::fs::read_dir(server.state_root().join("feishu_connections"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![format!("{}.json", server.connection_id)]);
    }

    #[test]
    fn a_reused_refresh_token_is_terminal_and_a_lost_answer_is_settled_by_one_request() {
        let server = MockFeishuServer::new();
        let auth = authorization(&server, MockFeishuServer::app());
        server.runtime().block_on(async {
            // The answer to the first refresh is lost after Feishu rotated.
            server.state.lock().unwrap().lose_refresh_answer_once = true;
            let error = auth.header().await.unwrap_err();
            assert_eq!(error, AuthError::Remote);
            assert_eq!(server.count("refresh"), 1);
            let document = server
                .connections
                .load(&server.connection_id)
                .unwrap()
                .unwrap();
            assert!(document.refresh_uncertain, "a lost answer leaves the flag");
            assert_eq!(
                document.refresh_token.expose(),
                MockFeishuServer::FIRST_REFRESH
            );
            let health = auth.health().unwrap();
            assert_eq!(health.state, ConnectionState::Unreachable);
            assert_eq!(health.code, "feishu_unreachable");
            assert!(!health.suspends());
            // The settling request presents the stored token once; Feishu
            // says it was used, so the earlier request landed.
            let error = auth.header().await.unwrap_err();
            assert_eq!(error, AuthError::Reconnect);
            assert_eq!(server.count("refresh"), 2);
            let health = auth.health().unwrap();
            assert_eq!(health.state, ConnectionState::ReconnectRequired);
            assert_eq!(health.code, "feishu_reconnect_required");
            assert!(health.suspends());
            let evidence = health.evidence.clone().unwrap();
            assert_eq!(evidence.endpoint, "oauth/v3/token");
            assert_eq!(evidence.http_status, Some(400));
            assert_eq!(evidence.error_tag, Some("20073"));
            let described = health.describe();
            assert!(
                described.contains("oauth/v3/token, HTTP 400, 20073"),
                "{described}"
            );
            assert!(
                !described.contains("refresh-1") && !described.contains("secret-canary"),
                "{described}"
            );
        });
    }

    #[test]
    fn a_lost_answer_followed_by_a_normal_answer_rotates_once() {
        let server = MockFeishuServer::new();
        let auth = authorization(&server, MockFeishuServer::app());
        server.runtime().block_on(async {
            // The request never reached Feishu: definitive, the flag clears.
            server.state.lock().unwrap().drop_refresh_before_rotate_once = true;
            assert_eq!(auth.header().await.unwrap_err(), AuthError::Remote);
            let document = server
                .connections
                .load(&server.connection_id)
                .unwrap()
                .unwrap();
            assert!(document.refresh_uncertain, "a server error may have landed");
            auth.header().await.unwrap();
            assert_eq!(server.count("refresh"), 2);
            let document = server
                .connections
                .load(&server.connection_id)
                .unwrap()
                .unwrap();
            assert!(!document.refresh_uncertain);
            assert_eq!(document.refresh_token.expose(), "refresh-2");
        });
    }

    #[test]
    fn refused_refreshes_reauthorization_limit_and_missing_documents_record_reconnect() {
        let server = MockFeishuServer::new();
        let auth = authorization(&server, MockFeishuServer::app());
        server.runtime().block_on(async {
            server.state.lock().unwrap().deny_refresh = true;
            assert_eq!(auth.header().await.unwrap_err(), AuthError::Reconnect);
            let health = auth.health().unwrap();
            assert_eq!(health.state, ConnectionState::ReconnectRequired);
            assert_eq!(
                health.evidence.as_ref().unwrap().error_tag,
                Some("invalid_grant")
            );
            assert_eq!(server.count("refresh"), 1);
            let document = server
                .connections
                .load(&server.connection_id)
                .unwrap()
                .unwrap();
            assert!(!document.refresh_uncertain, "a refusal is definitive");
            server.state.lock().unwrap().deny_refresh = false;

            // Mallard's App Secret rotated under an old build: the app is
            // refused, not the user, and nothing suspends the storage.
            let stale =
                FeishuApp::new("cli_test", SecretValue::parse("another-secret").unwrap()).unwrap();
            let rotated = authorization(&server, stale);
            assert_eq!(rotated.header().await.unwrap_err(), AuthError::AppRejected);
            assert_eq!(rotated.health().unwrap().code, "feishu_app_rejected");
            assert!(!rotated.health().unwrap().suspends());
            assert!(rotated
                .health()
                .unwrap()
                .describe()
                .contains("Update Mallard"));

            // Past the reauthorization date nothing is sent; a refresh answer
            // that would extend it never does.
            let mut document = server
                .connections
                .load(&server.connection_id)
                .unwrap()
                .unwrap();
            document.reauthorize_at = now_secs() + 100;
            server.connections.save(&document).unwrap();
            let before = server.count("refresh");
            auth.header().await.unwrap();
            assert_eq!(server.count("refresh"), before + 1);
            let document = server
                .connections
                .load(&server.connection_id)
                .unwrap()
                .unwrap();
            assert!(document.reauthorize_at <= now_secs() + 100);
            document_past_limit(&server);
            auth.invalidate().await;
            assert_eq!(auth.header().await.unwrap_err(), AuthError::Reconnect);
            assert_eq!(server.count("refresh"), before + 1, "nothing was sent");
            assert!(auth.health().unwrap().describe().contains("fresh sign-in"));

            // A missing document reads as reconnect required.
            server.connections.remove(&server.connection_id).unwrap();
            auth.invalidate().await;
            assert_eq!(auth.header().await.unwrap_err(), AuthError::Reconnect);
            assert!(auth
                .health()
                .unwrap()
                .describe()
                .contains("holds no Feishu sign-in"));
        });
    }

    fn document_past_limit(server: &MockFeishuServer) {
        let mut document = server
            .connections
            .load(&server.connection_id)
            .unwrap()
            .unwrap();
        document.reauthorize_at = 1;
        server.connections.save(&document).unwrap();
    }

    #[test]
    fn a_document_write_failure_fails_the_request_and_keeps_the_grant() {
        let server = MockFeishuServer::new();
        let auth = authorization(&server, MockFeishuServer::app());
        server.runtime().block_on(async {
            auth.header().await.unwrap();
            auth.cached.lock().await.as_mut().unwrap().expires_at = now_secs();
            let path = server.document_path();
            let refreshes = server.count("refresh");
            // The write before the request fails: nothing is sent.
            crate::persistence::test_support::fail_next_json_write(&path, false);
            assert_eq!(auth.header().await.unwrap_err(), AuthError::NotSaved);
            assert_eq!(server.count("refresh"), refreshes);
            assert_eq!(auth.health().unwrap().code, "feishu_connection_not_saved");
            assert!(!auth.health().unwrap().suspends());
            // The write after the answer fails: the rotated token is in
            // memory only, this request fails, and the grant stays.
            server.fail_document_write_after_next_refresh();
            assert_eq!(auth.header().await.unwrap_err(), AuthError::NotSaved);
            server.restore_documents();
            assert_eq!(server.count("refresh"), refreshes + 1);
            assert!(auth.cached.lock().await.is_some(), "the grant stays");
            auth.header().await.unwrap();
            assert_eq!(
                server.count("refresh"),
                refreshes + 1,
                "the next request uses it"
            );
            // The document still holds the token Feishu already consumed,
            // marked uncertain: the next launch's settling request ends in
            // a reconnect, never in a blind reuse that looks like success.
            let document = server
                .connections
                .load(&server.connection_id)
                .unwrap()
                .unwrap();
            assert!(document.refresh_uncertain);
        });
    }

    #[test]
    fn user_info_returns_a_bounded_name_and_open_id() {
        let server = MockFeishuServer::new();
        let client = server.client();
        let token = server.access_token();
        let account = server.runtime().block_on(client.user_info(&token)).unwrap();
        assert_eq!(account.name, "Test User");
        assert_eq!(account.id, "ou_fixture");
        assert!(server
            .runtime()
            .block_on(client.user_info(&SecretValue::parse("access-unknown").unwrap()))
            .is_err());
    }
}
