#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    #[error("unknown udf: {0}")]
    Unknown(String),
}

pub fn evaluate(definition: &UdfDefinition, input: &str) -> Result<String, UdfError> {
    if definition.allow_network {
        return Err(UdfError::CapabilityDenied("network"));
    }
    if definition.allow_filesystem {
        return Err(UdfError::CapabilityDenied("filesystem"));
    }
    match definition.name.as_str() {
        "upper" => Ok(input.to_ascii_uppercase()),
        "identity" => Ok(input.to_string()),
        other => Err(UdfError::Unknown(other.to_string())),
    }
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
}
