#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_udf::{MAX_UDF_INPUT_BYTES, UdfDefinition, UdfError, UdfRuntime, evaluate};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, SandboxError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SandboxError {
    #[error("sandbox denied capability: {0}")]
    CapabilityDenied(&'static str),
    #[error("invalid sandbox request: {0}")]
    InvalidRequest(&'static str),
    #[error("udf failed: {0}")]
    Udf(String),
}

impl From<UdfError> for SandboxError {
    fn from(error: UdfError) -> Self {
        match error {
            UdfError::CapabilityDenied(capability) => Self::CapabilityDenied(capability),
            other => Self::Udf(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdfRequest {
    pub name: String,
    pub runtime: UdfRuntime,
    pub source: String,
    pub input: String,
    pub allow_network: bool,
    pub allow_filesystem: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdfResponse {
    pub runtime: UdfRuntime,
    pub output: String,
}

pub trait SandboxRuntime: Send + Sync {
    fn runtime_name(&self) -> &'static str;
    fn run(&self, request: UdfRequest) -> Result<UdfResponse>;
}

#[derive(Clone, Debug, Default)]
pub struct LocalUdfSandbox;

impl SandboxRuntime for LocalUdfSandbox {
    fn runtime_name(&self) -> &'static str {
        "local-tdw-udf"
    }

    fn run(&self, request: UdfRequest) -> Result<UdfResponse> {
        validate_request(&request)?;
        let definition = UdfDefinition {
            name: request.name,
            runtime: request.runtime,
            source: request.source,
            allow_network: request.allow_network,
            allow_filesystem: request.allow_filesystem,
        };
        let output = evaluate(&definition, &request.input)?;
        Ok(UdfResponse {
            runtime: definition.runtime,
            output,
        })
    }
}

pub fn validate_request(request: &UdfRequest) -> Result<()> {
    if request.name.trim().is_empty() {
        return Err(SandboxError::InvalidRequest("name"));
    }
    if request.source.trim().is_empty() {
        return Err(SandboxError::InvalidRequest("source"));
    }
    if request.input.len() > MAX_UDF_INPUT_BYTES {
        return Err(SandboxError::InvalidRequest("input"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_sandbox_runs_existing_udf_contract() {
        let sandbox = LocalUdfSandbox;
        let response = sandbox
            .run(UdfRequest {
                name: "upper".to_string(),
                runtime: UdfRuntime::Wasm,
                source: "upper(input)".to_string(),
                input: "aapl".to_string(),
                allow_network: false,
                allow_filesystem: false,
            })
            .unwrap_or_else(|error| panic!("udf should run: {error}"));

        assert_eq!(response.output, "AAPL");
        assert_eq!(response.runtime, UdfRuntime::Wasm);
    }

    #[test]
    fn local_sandbox_preserves_denied_capabilities() {
        let sandbox = LocalUdfSandbox;
        let denied = sandbox.run(UdfRequest {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper(input)".to_string(),
            input: "aapl".to_string(),
            allow_network: true,
            allow_filesystem: false,
        });

        assert_eq!(denied, Err(SandboxError::CapabilityDenied("network")));
    }

    #[test]
    fn local_sandbox_rejects_empty_source_before_dispatch() {
        let sandbox = LocalUdfSandbox;
        let rejected = sandbox.run(UdfRequest {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: " ".to_string(),
            input: "aapl".to_string(),
            allow_network: false,
            allow_filesystem: false,
        });

        assert_eq!(rejected, Err(SandboxError::InvalidRequest("source")));
    }
}
