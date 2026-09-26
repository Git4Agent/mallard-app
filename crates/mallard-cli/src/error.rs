use std::fmt;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_INVALID_INPUT: i32 = 2;
pub const EXIT_CONFLICT: i32 = 3;
pub const EXIT_REMOTE_FAILURE: i32 = 4;
pub const EXIT_PERSISTENCE_FAILURE: i32 = 5;

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

    pub fn remote(code: &'static str, message: &'static str) -> Self {
        Self::new(EXIT_REMOTE_FAILURE, code, message)
    }

    pub fn from_setup(error: mallard_application::storage_setup::StorageSetupError) -> Self {
        Self::new(error.kind().exit_code(), error.code(), error.to_string())
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
}
