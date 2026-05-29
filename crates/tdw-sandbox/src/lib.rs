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
    /// Optional per-request WASM resource limits. Applies only to the
    /// `Wasm` runtime (ignored otherwise). Serde-default + skip so existing
    /// `udf.run` payloads keep deserializing/serializing unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_limits: Option<WasmLimitsRequest>,
}

/// Per-request override for the WASM runtime resource limits.
///
/// Each `None` field falls back to the runtime default. Provided values are a
/// request to **tighten** the limit: they are clamped to the runtime default
/// ceiling so a caller can run an untrusted UDF with a smaller fuel/memory
/// budget, but can never raise a limit above the built-in maximum (which would
/// otherwise be a DoS lever). See [`resolve_wasm_limits`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasmLimitsRequest {
    /// Max fuel (≈ executed bytecode ops) before the call traps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuel: Option<u64>,
    /// Max linear memory, in bytes, the instance may allocate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<usize>,
    /// Max number of linear memories the instance may define.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memories: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UdfResponse {
    pub runtime: UdfRuntime,
    pub output: String,
}

pub trait SandboxRuntime: Send + Sync {
    fn runtime_name(&self) -> &'static str;
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
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

/// Route a `UdfRuntime::Wasm` request to the WASM runtime.
///
/// `name` is the exported function name. A real module is carried as **base64
/// in `source`**: if `source` base64-decodes to bytes beginning with the wasm
/// magic (`\0asm`), it is executed through the hardened `wasmi` string ABI
/// (`execute_wasm_string`) under the request's [`WasmLimits`] (per-request
/// override clamped to the runtime ceiling — see [`resolve_wasm_limits`]; fuel,
/// memory caps, deny-by-default imports). Otherwise the request falls back to
/// the deterministic fixture interpreter (`name` as the export), preserving the
/// prior contract for non-wasm source.
///
/// Network and filesystem capabilities are denied first — the sandbox contract
/// is unchanged.
#[cfg(feature = "udf-wasm")]
fn run_wasm(request: &UdfRequest) -> Result<UdfResponse> {
    use base64::Engine as _;
    use tdw_udf_wasm::{WasmUdfError, WasmUdfRuntime};

    if request.allow_network {
        return Err(SandboxError::CapabilityDenied("network"));
    }
    if request.allow_filesystem {
        return Err(SandboxError::CapabilityDenied("filesystem"));
    }

    // `name` is a validated identifier (alphanumeric + `_` / `-`), so it always
    // passes `is_export_name`.
    let func = request.name.as_str();
    let rt = WasmUdfRuntime::new();
    let limits = resolve_wasm_limits(request.wasm_limits.as_ref());

    // Real module path: base64 in `source`, gated on the wasm magic so plain
    // (non-wasm) source stays on the fixture path below.
    if let Ok(module) = base64::engine::general_purpose::STANDARD.decode(request.source.as_bytes())
        && module.starts_with(&[0x00, 0x61, 0x73, 0x6d])
    {
        return rt
            .execute_wasm_string(&module, func, &request.input, limits)
            .map(|output| UdfResponse {
                runtime: UdfRuntime::Wasm,
                output,
            })
            .map_err(|error| SandboxError::Udf(error.to_string()));
    }

    // Fixture fallback: minimal valid WASM header (magic + version).
    let wasm_stub: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
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

/// Resolve the effective [`WasmLimits`] for a request.
///
/// Starts from `WasmLimits::default()` (the runtime ceiling). For every field
/// the caller supplied, the value is clamped down with `min` so a request can
/// only ever *tighten* a limit, never raise it above the built-in maximum.
/// Absent fields keep the default. This makes per-request limits a safe budget
/// knob for untrusted UDFs rather than a DoS lever.
#[cfg(feature = "udf-wasm")]
fn resolve_wasm_limits(request: Option<&WasmLimitsRequest>) -> tdw_udf_wasm::WasmLimits {
    let ceiling = tdw_udf_wasm::WasmLimits::default();
    let Some(request) = request else {
        return ceiling;
    };
    tdw_udf_wasm::WasmLimits {
        fuel: request.fuel.map_or(ceiling.fuel, |v| v.min(ceiling.fuel)),
        max_memory_bytes: request
            .max_memory_bytes
            .map_or(ceiling.max_memory_bytes, |v| {
                v.min(ceiling.max_memory_bytes)
            }),
        max_memories: request
            .max_memories
            .map_or(ceiling.max_memories, |v| v.min(ceiling.max_memories)),
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
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

#[cfg(all(test, feature = "udf-wasm"))]
mod wasm_routing_tests {
    use super::*;
    use base64::Engine as _;

    const ECHO: &str = r#"(module
        (memory (export "memory") 1)
        (global $bump (mut i32) (i32.const 1024))
        (func (export "alloc") (param $len i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $len)))
            (local.get $ptr))
        (func (export "echo") (param $ptr i32) (param $len i32) (result i64)
            (i64.or
                (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
                (i64.extend_i32_u (local.get $len)))))"#;

    fn b64_wasm(wat_text: &str) -> String {
        let module =
            wat::parse_str(wat_text).unwrap_or_else(|error| panic!("wat compiles: {error}"));
        base64::engine::general_purpose::STANDARD.encode(module)
    }

    fn wasm_request(name: &str, source: String, input: &str) -> UdfRequest {
        UdfRequest {
            name: name.to_string(),
            runtime: UdfRuntime::Wasm,
            source,
            input: input.to_string(),
            allow_network: false,
            allow_filesystem: false,
            wasm_limits: None,
        }
    }

    #[test]
    fn routes_base64_wasm_module_through_string_abi() {
        let response = LocalUdfSandbox
            .run(wasm_request("echo", b64_wasm(ECHO), "hello world"))
            .unwrap_or_else(|error| panic!("wasm echo should run: {error}"));
        assert_eq!(response.runtime, UdfRuntime::Wasm);
        assert_eq!(response.output, "hello world");
    }

    #[test]
    fn non_wasm_source_falls_back_to_fixture() {
        // `source` is not base64-wasm, so the fixture handles it via `name`.
        let response = LocalUdfSandbox
            .run(wasm_request(
                "upper",
                "plain udf source".to_string(),
                "aapl",
            ))
            .unwrap_or_else(|error| panic!("fixture should run: {error}"));
        assert_eq!(response.output, "AAPL");
    }

    #[test]
    fn fuel_exhaustion_maps_to_udf_error() {
        let spin = r#"(module
            (memory (export "memory") 1)
            (func (export "alloc") (param $len i32) (result i32) (i32.const 0))
            (func (export "spin") (param $ptr i32) (param $len i32) (result i64)
                (loop (br 0)) (i64.const 0)))"#;
        let result = LocalUdfSandbox.run(wasm_request("spin", b64_wasm(spin), "x"));
        assert!(matches!(result, Err(SandboxError::Udf(_))));
    }

    #[test]
    fn network_capability_denied_before_wasm() {
        let mut request = wasm_request("echo", b64_wasm(ECHO), "x");
        request.allow_network = true;
        assert_eq!(
            LocalUdfSandbox.run(request),
            Err(SandboxError::CapabilityDenied("network"))
        );
    }

    #[test]
    fn resolve_wasm_limits_defaults_when_absent() {
        assert_eq!(
            resolve_wasm_limits(None),
            tdw_udf_wasm::WasmLimits::default()
        );
    }

    #[test]
    fn resolve_wasm_limits_clamps_over_ceiling_and_keeps_unset_fields() {
        let ceiling = tdw_udf_wasm::WasmLimits::default();
        // Over-ceiling values are clamped down (no DoS lever); the unset
        // `max_memories` field keeps the default.
        let resolved = resolve_wasm_limits(Some(&WasmLimitsRequest {
            fuel: Some(u64::MAX),
            max_memory_bytes: Some(usize::MAX),
            max_memories: None,
        }));
        assert_eq!(resolved.fuel, ceiling.fuel);
        assert_eq!(resolved.max_memory_bytes, ceiling.max_memory_bytes);
        assert_eq!(resolved.max_memories, ceiling.max_memories);
    }

    #[test]
    fn resolve_wasm_limits_allows_tightening_below_ceiling() {
        let resolved = resolve_wasm_limits(Some(&WasmLimitsRequest {
            fuel: Some(1_000),
            max_memory_bytes: Some(64 * 1024),
            max_memories: Some(1),
        }));
        assert_eq!(resolved.fuel, 1_000);
        assert_eq!(resolved.max_memory_bytes, 64 * 1024);
        assert_eq!(resolved.max_memories, 1);
    }

    #[test]
    fn per_request_low_fuel_traps_a_module_that_runs_under_default() {
        // ECHO runs fine under the default fuel budget...
        let ok = LocalUdfSandbox.run(wasm_request("echo", b64_wasm(ECHO), "hello"));
        assert!(ok.is_ok(), "echo should run under default fuel: {ok:?}");

        // ...but a tiny per-request fuel budget traps it, proving the override
        // is threaded all the way through to execution.
        let mut tight = wasm_request("echo", b64_wasm(ECHO), "hello");
        tight.wasm_limits = Some(WasmLimitsRequest {
            fuel: Some(1),
            ..WasmLimitsRequest::default()
        });
        assert!(matches!(
            LocalUdfSandbox.run(tight),
            Err(SandboxError::Udf(_))
        ));
    }

    #[test]
    fn udf_request_omits_wasm_limits_when_absent_and_round_trips_when_present() {
        // Back-compat: a payload without `wasm_limits` deserializes (serde
        // default) and re-serializes without the field.
        let json = r#"{"name":"echo","runtime":"Wasm","source":"","input":"x","allow_network":false,"allow_filesystem":false}"#;
        let request: UdfRequest =
            serde_json::from_str(json).unwrap_or_else(|error| panic!("deserialize: {error}"));
        assert!(request.wasm_limits.is_none());
        let reserialized =
            serde_json::to_string(&request).unwrap_or_else(|error| panic!("serialize: {error}"));
        assert!(!reserialized.contains("wasm_limits"));

        // A payload carrying limits round-trips them.
        let with_limits = UdfRequest {
            wasm_limits: Some(WasmLimitsRequest {
                fuel: Some(500),
                ..WasmLimitsRequest::default()
            }),
            ..request
        };
        let encoded = serde_json::to_string(&with_limits)
            .unwrap_or_else(|error| panic!("serialize: {error}"));
        let decoded: UdfRequest =
            serde_json::from_str(&encoded).unwrap_or_else(|error| panic!("deserialize: {error}"));
        assert_eq!(
            decoded.wasm_limits.and_then(|limits| limits.fuel),
            Some(500)
        );
    }
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
                wasm_limits: None,
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
            wasm_limits: None,
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
            wasm_limits: None,
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
            wasm_limits: None,
        });
        assert_eq!(bad_name, Err(SandboxError::InvalidRequest("name")));

        let oversized_source = sandbox.run(UdfRequest {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "x".repeat(MAX_UDF_SOURCE_BYTES + 1),
            input: "aapl".to_string(),
            allow_network: false,
            allow_filesystem: false,
            wasm_limits: None,
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
                wasm_limits: None,
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
            wasm_limits: None,
        });

        assert_eq!(denied, Err(SandboxError::CapabilityDenied("network")));
    }
}
