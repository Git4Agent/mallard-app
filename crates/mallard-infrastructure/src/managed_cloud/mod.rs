//! Mallard Cloud: the managed storage service behind the bundle engine
//! (architecture section 5). This module holds the HTTP client, the object
//! store, the session-view builder, and the desktop sign-in; the adapter
//! lives with the other storage providers.
pub mod address;
pub mod app_link;
pub mod client;
pub mod diagnostics;
pub mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod stub;
pub mod views;

use crate::loopback_browser::{LoopbackCallback, LoopbackErrors, LoopbackListener};
use client::{ManagedCloudClient, User};
use mallard_domain::SecretValue;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use zeroize::Zeroizing;

pub use client::{Error, ErrorKind};
pub use store::ManagedObjectStore;

/// The production service. Every release build speaks to this origin only.
pub const PRODUCTION_ORIGIN: &str = "https://mallard-ai.com";

/// The origin a new storage points at: production, or, in a development
/// build, `MALLARD_MANAGED_CLOUD_ORIGIN` when it names a valid origin.
pub fn default_origin() -> String {
    if cfg!(debug_assertions) {
        if let Some(origin) = std::env::var("MALLARD_MANAGED_CLOUD_ORIGIN")
            .ok()
            .filter(|origin| client::parse_origin(origin).is_ok())
        {
            return origin;
        }
    }
    PRODUCTION_ORIGIN.to_string()
}

/// Whether a saved origin is one this build may talk to: production
/// always; anything well-formed in a development build.
pub fn origin_allowed(origin: &str) -> bool {
    client::parse_origin(origin).is_ok() && (cfg!(debug_assertions) || origin == PRODUCTION_ORIGIN)
}

/// The loopback callback the service redirects the browser to
/// (`http://127.0.0.1:{port}/mallard/callback?state&grant`).
pub const MANAGED_CLOUD_CALLBACK: LoopbackCallback = LoopbackCallback {
    path: "/mallard/callback",
    result_key: "grant",
    completion_path: "/mallard/complete",
    title: "Mallard Cloud sign-in",
    received_message: "Signed in. Return to Mallard and wait for setup to finish.",
    failed_message: "The sign-in was not completed. Return to Mallard.",
    errors: LoopbackErrors {
        invalid: (
            "managed_cloud_callback_invalid",
            "Mallard Cloud returned an invalid sign-in callback. Sign in again.",
        ),
        unavailable: (
            "managed_cloud_callback_unavailable",
            "Mallard could not open a loopback port for the sign-in. Close the other setup attempt and try again.",
        ),
        browser_failed: (
            "managed_cloud_browser_failed",
            "Mallard could not hand over the sign-in link.",
        ),
        cancelled: ("managed_cloud_setup_cancelled", "Mallard Cloud setup was cancelled."),
        denied: (
            "managed_cloud_consent_denied",
            "GitHub sign-in was not approved. Setup stopped.",
        ),
        timeout: (
            "managed_cloud_authorization_timeout",
            "The sign-in timed out. Sign in again when you are ready.",
        ),
    },
};

/// PKCE material for one desktop sign-in: the verifier stays in memory,
/// the S256 challenge goes to the service.
pub struct Pkce {
    pub verifier: Zeroizing<String>,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Result<Self, String> {
        use sha2::{Digest, Sha256};
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(bytes.as_mut()).map_err(|_| "random verifier".to_string())?;
        let verifier = Zeroizing::new(mallard_domain::hex_lower(bytes.as_ref()));
        let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
        Ok(Self {
            verifier,
            challenge,
        })
    }
}

fn base64url(bytes: &[u8]) -> String {
    aws_smithy_types::base64::encode(bytes)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

/// What a completed sign-in yields.
pub struct SignedIn {
    pub token: SecretValue,
    pub expires_at: u64,
    pub user: User,
}

/// The whole desktop sign-in: bind a loopback port, start the attempt,
/// hand the link over, wait for the callback, exchange the grant. The
/// link is never opened here.
pub async fn sign_in(
    origin: &str,
    on_link: impl FnOnce(&url::Url) -> Result<(), ()> + Send + 'static,
    cancelled: std::sync::Arc<AtomicBool>,
) -> Result<SignedIn, SignInError> {
    let client = ManagedCloudClient::new(origin, None).map_err(SignInError::Local)?;
    let pkce = Pkce::generate().map_err(SignInError::Local)?;
    let listener = LoopbackListener::bind(&MANAGED_CLOUD_CALLBACK, 0)
        .map_err(|error| SignInError::Callback(error.code()))?;
    let started = client
        .desktop_start(&pkce.challenge, listener.port())
        .await
        .map_err(SignInError::Service)?;
    let url = url::Url::parse(&started.authorize_url)
        .ok()
        .filter(|url| {
            url.as_str().len() <= 8192
                && (url.scheme() == "https" || (cfg!(debug_assertions) && url.scheme() == "http"))
                && url.username().is_empty()
                && url.password().is_none()
        })
        .ok_or(SignInError::Callback("managed_cloud_callback_invalid"))?;
    let state = started.state.clone();
    // Always join the receiver: cancellation must release the port.
    let grant = tokio::task::spawn_blocking(move || {
        listener.receive(
            &MANAGED_CLOUD_CALLBACK,
            &url,
            &state,
            on_link,
            Duration::from_secs(180),
            &cancelled,
        )
    })
    .await
    .map_err(|_| SignInError::Local("the sign-in receiver stopped".to_string()))?
    .map_err(|error| SignInError::Callback(error.code()))?;
    let exchanged = client
        .desktop_exchange(&started.state, &grant, &pkce.verifier)
        .await
        .map_err(SignInError::Service)?;
    let token = SecretValue::parse(exchanged.token.as_str())
        .map_err(|_| SignInError::Local("the issued token is malformed".to_string()))?;
    Ok(SignedIn {
        token,
        expires_at: exchanged.expires_at,
        user: exchanged.user,
    })
}

#[derive(Debug)]
pub enum SignInError {
    /// A loopback failure, by the callback's fixed code.
    Callback(&'static str),
    Service(Error),
    Local(String),
}

impl SignInError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Callback(code) => code,
            Self::Service(error) => match error.kind {
                ErrorKind::Auth => "managed_cloud_sign_in_rejected",
                ErrorKind::Permission => "managed_cloud_permission_denied",
                ErrorKind::Unreachable | ErrorKind::Unknown => "managed_cloud_unreachable",
                _ => "managed_cloud_service_rejected",
            },
            Self::Local(_) => "managed_cloud_setup_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_the_s256_of_the_verifier() {
        use sha2::{Digest, Sha256};
        let pkce = Pkce::generate().unwrap();
        assert_eq!(pkce.verifier.len(), 64);
        assert_eq!(pkce.challenge.len(), 43);
        assert_eq!(
            pkce.challenge,
            base64url(&Sha256::digest(pkce.verifier.as_bytes()))
        );
        assert!(pkce
            .challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    }

    #[test]
    fn origins_are_gated_by_build() {
        assert!(origin_allowed(PRODUCTION_ORIGIN));
        assert!(!origin_allowed("https://mallard-ai.com/"));
        assert_eq!(
            origin_allowed("http://localhost:8787"),
            cfg!(debug_assertions)
        );
        assert_eq!(
            default_origin() == PRODUCTION_ORIGIN,
            std::env::var("MALLARD_MANAGED_CLOUD_ORIGIN").is_err() || !cfg!(debug_assertions)
        );
    }
}
