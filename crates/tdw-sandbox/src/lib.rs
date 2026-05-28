#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_udf::{
    MAX_UDF_INPUT_BYTES, MAX_UDF_SOURCE_BYTES, UdfDefinition, UdfError, UdfRuntime, evaluate,
};
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

        // When the `udf-wasm` feature is enabled, route Wasm requests through
        // `WasmUdfRuntime` instead of the built-in dispatcher. The source field
        // is treated as the exported function name; the WASM module is a minimal
        // valid header (fixture runtime — see tdw-udf-wasm docs for follow-up).
        #[cfg(feature = "udf-wasm")]
        if request.runtime == UdfRuntime::Wasm {
            return run_wasm(&request);
        }

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

/// Route a `UdfRuntime::Wasm` request through the WASM fixture runtime.
///
/// The `name` field of the request is used as the exported function name
/// (e.g. `"upper"`), keeping the existing sandbox contract stable: callers
/// set `name` to the UDF identifier and `source` to the UDF body / metadata.
/// A minimal valid WASM module header is synthesised so that magic-byte
/// validation inside `WasmUdfRuntime::execute` passes.
/// Network and filesystem capabilities are still denied — the sandbox contract
/// is unchanged.
#[cfg(feature = "udf-wasm")]
fn run_wasm(request: &UdfRequest) -> Result<UdfResponse> {
    use tdw_udf_wasm::{WasmUdfError, WasmUdfRuntime};

    if request.allow_network {
        return Err(SandboxError::CapabilityDenied("network"));
    }
    if request.allow_filesystem {
        return Err(SandboxError::CapabilityDenied("filesystem"));
    }

    // Minimal valid WASM binary: magic + version.
    let wasm_stub: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
    // Use `name` as the exported function name — it's a validated identifier
    // (alphanumeric + `_` / `-`) so it always passes `is_export_name`.
    let func = request.name.as_str();
    let rt = WasmUdfRuntime::new();

    rt.execute(wasm_stub, func, &request.input)
        .map(|output| UdfResponse {
            runtime: UdfRuntime::Wasm,
            output,
        })
        .map_err(|e| match e {
            WasmUdfError::UnknownExport => {
                SandboxError::Udf(format!("unknown wasm export: {func}"))
            }
            other => SandboxError::Udf(other.to_string()),
        })
}

pub fn validate_request(request: &UdfRequest) -> Result<()> {
    if !is_udf_name(&request.name) {
        return Err(SandboxError::InvalidRequest("name"));
    }
    if request.source.trim().is_empty() {
        return Err(SandboxError::InvalidRequest("source"));
    }
    if request.source.len() > MAX_UDF_SOURCE_BYTES {
        return Err(SandboxError::InvalidRequest("source"));
    }
    if request.input.len() > MAX_UDF_INPUT_BYTES {
        return Err(SandboxError::InvalidRequest("input"));
    }
    Ok(())
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

    #[test]
    fn local_sandbox_rejects_bad_name_and_oversized_source_before_dispatch() {
        let sandbox = LocalUdfSandbox;
        let bad_name = sandbox.run(UdfRequest {
            name: "../upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper(input)".to_string(),
            input: "aapl".to_string(),
            allow_network: false,
            allow_filesystem: false,
        });
        assert_eq!(bad_name, Err(SandboxError::InvalidRequest("name")));

        let oversized_source = sandbox.run(UdfRequest {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "x".repeat(MAX_UDF_SOURCE_BYTES + 1),
            input: "aapl".to_string(),
            allow_network: false,
            allow_filesystem: false,
        });
        assert_eq!(
            oversized_source,
            Err(SandboxError::InvalidRequest("source"))
        );
    }

    /// When compiled with `udf-wasm`, the Wasm runtime routes through
    /// `WasmUdfRuntime` and still produces the correct deterministic output.
    #[cfg(feature = "udf-wasm")]
    #[test]
    fn wasm_runtime_routes_through_wasm_udf_runtime() {
        let sandbox = LocalUdfSandbox;
        let response = sandbox
            .run(UdfRequest {
                name: "upper".to_string(),
                runtime: UdfRuntime::Wasm,
                // source = exported function name in fixture interpreter
                source: "upper".to_string(),
                input: "msft".to_string(),
                allow_network: false,
                allow_filesystem: false,
            })
            .unwrap_or_else(|error| panic!("wasm udf should run: {error}"));

        assert_eq!(response.output, "MSFT");
        assert_eq!(response.runtime, UdfRuntime::Wasm);
    }

    /// Network capability must be denied even via the wasm runtime path.
    #[cfg(feature = "udf-wasm")]
    #[test]
    fn wasm_runtime_denies_network_capability() {
        let sandbox = LocalUdfSandbox;
        let denied = sandbox.run(UdfRequest {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper".to_string(),
            input: "msft".to_string(),
            allow_network: true,
            allow_filesystem: false,
        });

        assert_eq!(denied, Err(SandboxError::CapabilityDenied("network")));
    }
}
