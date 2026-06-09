#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_UDF_SOURCE_BYTES: usize = 16 * 1024;
pub const MAX_UDF_INPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UdfRuntime {
    JavaScript,
    Python,
    Wasm,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdfDefinition {
    pub name: String,
    pub runtime: UdfRuntime,
    pub source: String,
    pub allow_network: bool,
    pub allow_filesystem: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UdfError {
    #[error("udf sandbox denied capability: {0}")]
    CapabilityDenied(&'static str),
    #[error("invalid udf definition: {0}")]
    InvalidDefinition(&'static str),
    #[error("udf source too large")]
    SourceTooLarge,
    #[error("udf input too large")]
    InputTooLarge,
    #[error("unknown udf: {0}")]
    Unknown(String),
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn evaluate(definition: &UdfDefinition, input: &str) -> Result<String, UdfError> {
    validate_definition(definition)?;
    if input.len() > MAX_UDF_INPUT_BYTES {
        return Err(UdfError::InputTooLarge);
    }

    dispatch_builtin(definition, input)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_definition(definition: &UdfDefinition) -> Result<(), UdfError> {
    if !is_udf_name(&definition.name) {
        return Err(UdfError::InvalidDefinition("name"));
    }
    if definition.source.trim().is_empty() {
        return Err(UdfError::InvalidDefinition("source"));
    }
    if definition.source.len() > MAX_UDF_SOURCE_BYTES {
        return Err(UdfError::SourceTooLarge);
    }
    if definition.allow_network {
        return Err(UdfError::CapabilityDenied("network"));
    }
    if definition.allow_filesystem {
        return Err(UdfError::CapabilityDenied("filesystem"));
    }
    Ok(())
}

fn dispatch_builtin(definition: &UdfDefinition, input: &str) -> Result<String, UdfError> {
    match definition.name.as_str() {
        "upper" => Ok(input.to_ascii_uppercase()),
        "identity" => Ok(input.to_string()),
        other => Err(UdfError::Unknown(other.to_string())),
    }
}

fn is_udf_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn udf_sandbox_executes_allowed_function_and_denies_network() {
        let definition = UdfDefinition {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "(input) => input.toUpperCase()".to_string(),
            allow_network: false,
            allow_filesystem: false,
        };
        assert_eq!(
            evaluate(&definition, "aapl")
                .unwrap_or_else(|error| panic!("udf should evaluate: {error}")),
            "AAPL"
        );

        let denied = UdfDefinition {
            allow_network: true,
            ..definition
        };
        assert_eq!(
            evaluate(&denied, "aapl"),
            Err(UdfError::CapabilityDenied("network"))
        );
    }

    #[test]
    fn rejects_unsafe_definition_and_oversized_input() {
        let definition = UdfDefinition {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper(input)".to_string(),
            allow_network: false,
            allow_filesystem: false,
        };

        assert_eq!(
            validate_definition(&UdfDefinition {
                name: "../upper".to_string(),
                ..definition.clone()
            }),
            Err(UdfError::InvalidDefinition("name"))
        );
        assert_eq!(
            validate_definition(&UdfDefinition {
                source: " ".to_string(),
                ..definition.clone()
            }),
            Err(UdfError::InvalidDefinition("source"))
        );
        assert_eq!(
            evaluate(&definition, &"x".repeat(MAX_UDF_INPUT_BYTES + 1)),
            Err(UdfError::InputTooLarge)
        );
    }
}
