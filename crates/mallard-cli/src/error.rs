use std::fmt;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_INVALID_INPUT: i32 = 2;
pub const EXIT_CONFLICT: i32 = 3;
pub const EXIT_REMOTE_FAILURE: i32 = 4;
pub const EXIT_PERSISTENCE_FAILURE: i32 = 5;
pub const EXIT_CLEANUP_REQUIRED: i32 = 6;

pub(crate) struct CliError {
    exit_code: i32,
    code: &'static str,
    message: String,
    setup_id: Option<String>,
}

impl CliError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(EXIT_INVALID_INPUT, "cli_invalid_input", message, None)
    }

    pub fn persistence(message: impl Into<String>) -> Self {
        Self::new(
            EXIT_PERSISTENCE_FAILURE,
            "cli_persistence_failure",
            message,
            None,
        )
    }

    pub fn from_setup(error: mallard_application::storage_setup::StorageSetupError) -> Self {
        let exit_code = error.kind().exit_code();
        let code = error.code();
        let setup_id = error.setup_id().map(str::to_string);
        Self::new(exit_code, code, error.to_string(), setup_id)
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

    pub fn setup_id(&self) -> Option<&str> {
        self.setup_id.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        exit_code: i32,
        code: &'static str,
        message: impl Into<String>,
        setup_id: Option<&str>,
    ) -> Self {
        Self::new(exit_code, code, message, setup_id.map(str::to_string))
    }

    fn new(
        exit_code: i32,
        code: &'static str,
        message: impl Into<String>,
        setup_id: Option<String>,
    ) -> Self {
        debug_assert!(matches!(exit_code, 2..=6));
        Self {
            exit_code,
            code,
            message: message.into(),
            setup_id,
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
            .field("setup_id", &self.setup_id)
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
        assert_eq!(
            StorageSetupErrorKind::CleanupRequired.exit_code(),
            EXIT_CLEANUP_REQUIRED
        );
    }
}
