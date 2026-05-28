#![forbid(unsafe_code)]

pub const CRATE_NAME: &str = "tdw-udf-external";
pub const RUNTIME_NAME: &str = "external";
pub const MAX_TIMEOUT_MS: u64 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalUdfCommand {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalUdfError {
    EmptyName,
    InvalidCommand,
    InvalidArgument,
    InvalidTimeout,
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_command(command: &ExternalUdfCommand) -> Result<(), ExternalUdfError> {
    if command.name.trim().is_empty() {
        return Err(ExternalUdfError::EmptyName);
    }
    if !is_command_name(&command.command) {
        return Err(ExternalUdfError::InvalidCommand);
    }
    if command.args.iter().any(|arg| contains_shell_control(arg)) {
        return Err(ExternalUdfError::InvalidArgument);
    }
    if command.timeout_ms == 0 || command.timeout_ms > MAX_TIMEOUT_MS {
        return Err(ExternalUdfError::InvalidTimeout);
    }

    Ok(())
}

fn is_command_name(value: &str) -> bool {
    !value.is_empty()
        && !value
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
        && !contains_shell_control(value)
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn contains_shell_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() || matches!(character, ';' | '&' | '|' | '`'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_allowlisted_command_shape() {
        let command = ExternalUdfCommand {
            name: CRATE_NAME.to_string(),
            command: "tdw-udf-runner".to_string(),
            args: vec!["--runtime".to_string(), "wasm".to_string()],
            timeout_ms: 5_000,
        };

        assert_eq!(RUNTIME_NAME, "external");
        assert_eq!(validate_command(&command), Ok(()));
    }

    #[test]
    fn rejects_paths_shell_controls_and_unbounded_timeouts() {
        let command = ExternalUdfCommand {
            name: CRATE_NAME.to_string(),
            command: "../runner".to_string(),
            args: Vec::new(),
            timeout_ms: 5_000,
        };

        assert_eq!(
            validate_command(&command),
            Err(ExternalUdfError::InvalidCommand)
        );
        assert_eq!(
            validate_command(&ExternalUdfCommand {
                command: "tdw-runner".to_string(),
                args: vec!["symbol=AAPL;rm".to_string()],
                ..command.clone()
            }),
            Err(ExternalUdfError::InvalidArgument)
        );
        assert_eq!(
            validate_command(&ExternalUdfCommand {
                command: "tdw-runner".to_string(),
                args: Vec::new(),
                timeout_ms: MAX_TIMEOUT_MS + 1,
                ..command
            }),
            Err(ExternalUdfError::InvalidTimeout)
        );
    }
}
