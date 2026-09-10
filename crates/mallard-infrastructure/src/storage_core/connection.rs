//! Runtime connection health of one opened store. It lives beside the
//! credential the transport presents, is never persisted, and carries only
//! allowlisted evidence: a route literal, an HTTP status, a fixed error tag,
//! and a validated request ID. Never a body, never a token.
use mallard_domain::{now_secs, StorageId};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use tokio::sync::broadcast;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connected,
    /// The saved credential was rejected; only a new credential recovers.
    ReconnectRequired,
    /// A manual access token expired; only a replacement recovers.
    TokenExpired,
    /// The account or grant lacks a permission; only new consent recovers.
    PermissionDenied,
    /// The service did not answer. The next request may succeed.
    Unreachable,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ConnectionEvidence {
    /// The exact route literal the transport sent, never a computed string.
    pub endpoint: &'static str,
    /// Absent when no response arrived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// One of the provider's fixed error identifiers, or nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_tag: Option<&'static str>,
    /// The provider's request ID, kept only when it is short and plain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl ConnectionEvidence {
    /// Accept a request ID only when it is at most 64 ASCII letters, digits,
    /// or hyphens; anything else is dropped rather than sanitised.
    pub fn request_id(value: Option<&[u8]>) -> Option<String> {
        let value = value?;
        if value.is_empty()
            || value.len() > 64
            || !value
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        {
            return None;
        }
        String::from_utf8(value.to_vec()).ok()
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ConnectionHealth {
    pub state: ConnectionState,
    pub code: &'static str,
    pub message: &'static str,
    pub observed_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<ConnectionEvidence>,
}

impl ConnectionHealth {
    pub fn connected() -> Self {
        Self {
            state: ConnectionState::Connected,
            code: "connected",
            message: "Connected.",
            observed_at: now_secs(),
            evidence: None,
        }
    }

    pub fn failed(
        state: ConnectionState,
        code: &'static str,
        message: &'static str,
        evidence: Option<ConnectionEvidence>,
    ) -> Self {
        Self {
            state,
            code,
            message,
            observed_at: now_secs(),
            evidence,
        }
    }

    /// Whether the store should refuse further requests until its
    /// credential changes or a probe succeeds.
    pub fn suspends(&self) -> bool {
        matches!(
            self.state,
            ConnectionState::ReconnectRequired
                | ConnectionState::TokenExpired
                | ConnectionState::PermissionDenied
        )
    }

    /// The message with its evidence, for notices and the activity log.
    pub fn describe(&self) -> String {
        let Some(evidence) = &self.evidence else {
            return self.message.to_owned();
        };
        let mut facts = vec![evidence.endpoint.to_owned()];
        facts.push(match evidence.http_status {
            Some(status) => format!("HTTP {status}"),
            None => "no response".to_owned(),
        });
        if let Some(tag) = evidence.error_tag {
            facts.push(tag.to_owned());
        }
        if let Some(id) = &evidence.request_id {
            facts.push(format!("request {id}"));
        }
        format!("{} ({})", self.message, facts.join(", "))
    }
}

/// The cell one transport records into. Cheap to read; written only when
/// the state actually changes.
#[derive(Default)]
pub struct ConnectionHealthCell(Mutex<Option<ConnectionHealth>>);

impl ConnectionHealthCell {
    pub fn get(&self) -> Option<ConnectionHealth> {
        self.0.lock().ok().and_then(|cell| cell.clone())
    }

    /// Record a failure. Returns whether the state changed.
    pub fn record(&self, health: ConnectionHealth) -> bool {
        let Ok(mut cell) = self.0.lock() else {
            return false;
        };
        let changed = cell.as_ref().map(|current| current.state) != Some(health.state);
        *cell = Some(health);
        changed
    }

    /// Record a success. Returns whether the state changed.
    pub fn record_connected(&self) -> bool {
        let Ok(mut cell) = self.0.lock() else {
            return false;
        };
        if cell
            .as_ref()
            .is_some_and(|current| current.state == ConnectionState::Connected)
        {
            return false;
        }
        *cell = Some(ConnectionHealth::connected());
        true
    }
}

/// Storage IDs whose connection state changed or was forgotten, for a
/// process-wide listener that tells the UI to re-read the listing. A send
/// with no receiver is dropped; a lagged receiver simply re-reads.
pub fn connection_events() -> &'static broadcast::Sender<StorageId> {
    static EVENTS: OnceLock<broadcast::Sender<StorageId>> = OnceLock::new();
    EVENTS.get_or_init(|| broadcast::channel(64).0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_short_plain_ascii_or_dropped() {
        assert_eq!(
            ConnectionEvidence::request_id(Some(b"abc-123")).as_deref(),
            Some("abc-123")
        );
        assert!(ConnectionEvidence::request_id(Some(b"")).is_none());
        assert!(ConnectionEvidence::request_id(Some(b"has space")).is_none());
        assert!(ConnectionEvidence::request_id(Some(&[b'a'; 65])).is_none());
        assert!(ConnectionEvidence::request_id(Some("é".as_bytes())).is_none());
        assert!(ConnectionEvidence::request_id(None).is_none());
    }

    #[test]
    fn describe_lists_only_allowlisted_facts_and_suspension_follows_state() {
        let health = ConnectionHealth::failed(
            ConnectionState::ReconnectRequired,
            "dropbox_reconnect_required",
            "Rejected.",
            Some(ConnectionEvidence {
                endpoint: "oauth2/token",
                http_status: Some(400),
                error_tag: Some("invalid_grant"),
                request_id: Some("req-1".into()),
            }),
        );
        assert_eq!(
            health.describe(),
            "Rejected. (oauth2/token, HTTP 400, invalid_grant, request req-1)"
        );
        assert!(health.suspends());
        let unreachable = ConnectionHealth::failed(
            ConnectionState::Unreachable,
            "dropbox_request_failed",
            "Unreachable.",
            Some(ConnectionEvidence {
                endpoint: "files/get_metadata",
                http_status: None,
                error_tag: None,
                request_id: None,
            }),
        );
        assert_eq!(
            unreachable.describe(),
            "Unreachable. (files/get_metadata, no response)"
        );
        assert!(!unreachable.suspends());
        assert!(!ConnectionHealth::connected().suspends());
        assert_eq!(ConnectionHealth::connected().describe(), "Connected.");
    }

    #[test]
    fn cell_reports_transitions_only() {
        let cell = ConnectionHealthCell::default();
        assert!(cell.get().is_none());
        assert!(cell.record_connected());
        assert!(!cell.record_connected());
        let failed = ConnectionHealth::failed(
            ConnectionState::Unreachable,
            "dropbox_request_failed",
            "Unreachable.",
            None,
        );
        assert!(cell.record(failed.clone()));
        assert!(!cell.record(failed));
        assert!(cell.record_connected());
        assert_eq!(cell.get().unwrap().state, ConnectionState::Connected);
    }
}
