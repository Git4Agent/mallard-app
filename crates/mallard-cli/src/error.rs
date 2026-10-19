use std::fmt;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_INVALID_INPUT: i32 = 2;
pub const EXIT_CONFLICT: i32 = 3;
pub const EXIT_REMOTE_FAILURE: i32 = 4;
pub const EXIT_PERSISTENCE_FAILURE: i32 = 5;

/// The sentences the services refuse a stale review with. A review that no
/// longer matches the storage is a conflict the agent resolves by reviewing
/// again, never by retrying the same apply.
const REVIEW_SUPERSEDED: &str = "The storage changed since this review. Compare again.";
const REVIEW_EXPIRED: &str = "This review is older than a day. Compare again.";

pub(crate) struct CliError {
    exit_code: i32,
    code: &'static str,
    message: String,
}

impl CliError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(EXIT_INVALID_INPUT, "cli_invalid_input", message)
    }

    pub fn persistence(message: impl Into<String>) -> Self {
        Self::new(EXIT_PERSISTENCE_FAILURE, "cli_persistence_failure", message)
    }

    pub fn invalid_code(code: &'static str, message: &'static str) -> Self {
        Self::new(EXIT_INVALID_INPUT, code, message)
    }

    /// Invalid input whose sentence is composed at the call site.
    pub fn invalid_with(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(EXIT_INVALID_INPUT, code, message)
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(EXIT_CONFLICT, code, message)
    }

    pub fn remote(code: &'static str, message: &'static str) -> Self {
        Self::new(EXIT_REMOTE_FAILURE, code, message)
    }

    pub fn from_setup(error: mallard_application::storage_setup::StorageSetupError) -> Self {
        Self::new(error.kind().exit_code(), error.code(), error.to_string())
    }

    /// A service's sentence, classified by what the agent should do next: a
    /// review or revision that no longer holds is a conflict (review again,
    /// re-read the revision); a name that nothing matches is invalid input;
    /// anything else failed against storage or a provider.
    pub fn from_service(message: String) -> Self {
        let lower = message.to_ascii_lowercase();
        if message.contains(REVIEW_SUPERSEDED) || message.contains(REVIEW_EXPIRED) {
            return Self::new(EXIT_CONFLICT, "review_superseded", message);
        }
        // A revision check or a "changed since you looked" sentence: the
        // caller re-reads and tries again with what it then sees.
        if (lower.contains("revision")
            && (lower.contains("expected")
                || lower.contains("changed")
                || lower.contains("stale")
                || lower.contains("mismatch")))
            || (lower.contains("changed") && (lower.contains("reload") || lower.contains("since")))
        {
            return Self::new(EXIT_CONFLICT, "revision_conflict", message);
        }
        if lower.starts_with("unknown ")
            || lower.contains("not found")
            || lower.contains("no such")
            || lower.contains("is not registered")
            || lower.contains("does not exist")
            || lower.contains("invalid")
            || lower.contains("must be")
        {
            return Self::new(EXIT_INVALID_INPUT, "invalid_input", message);
        }
        Self::new(EXIT_REMOTE_FAILURE, "operation_failed", message)
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    fn new(exit_code: i32, code: &'static str, message: impl Into<String>) -> Self {
        debug_assert!(matches!(exit_code, 2..=5));
        Self {
            exit_code,
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl fmt::Debug for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CliError")
            .field("exit_code", &self.exit_code)
            .field("code", &self.code)
            .field("message", &self.message)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mallard_application::storage_setup::StorageSetupErrorKind;

    #[test]
    fn application_error_kinds_use_the_public_cli_exit_contract() {
        assert_eq!(
            StorageSetupErrorKind::InvalidInput.exit_code(),
            EXIT_INVALID_INPUT
        );
        assert_eq!(StorageSetupErrorKind::Conflict.exit_code(), EXIT_CONFLICT);
        assert_eq!(
            StorageSetupErrorKind::RemoteFailure.exit_code(),
            EXIT_REMOTE_FAILURE
        );
        assert_eq!(
            StorageSetupErrorKind::PersistenceFailure.exit_code(),
            EXIT_PERSISTENCE_FAILURE
        );
    }

    #[test]
    fn service_sentences_are_classified_for_the_agent() {
        let superseded = CliError::from_service(format!("Pull refused: {REVIEW_SUPERSEDED}"));
        assert_eq!(superseded.exit_code(), EXIT_CONFLICT);
        assert_eq!(superseded.code(), "review_superseded");
        let stale =
            CliError::from_service("project revision changed: expected 3, found 4".to_string());
        assert_eq!(stale.exit_code(), EXIT_CONFLICT);
        assert_eq!(stale.code(), "revision_conflict");
        let unknown = CliError::from_service("unknown storage 'storage-x'".to_string());
        assert_eq!(unknown.exit_code(), EXIT_INVALID_INPUT);
        let remote = CliError::from_service("storage listing failed: timeout".to_string());
        assert_eq!(remote.exit_code(), EXIT_REMOTE_FAILURE);
        assert_eq!(remote.code(), "operation_failed");
    }
}
