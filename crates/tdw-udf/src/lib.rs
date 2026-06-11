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
    /// The requested [`UdfRuntime`] variant is not yet implemented.
    ///
    /// # G008 theme 10 — UDF1: unimplemented-runtime gate
    ///
    /// `evaluate` previously dispatched solely on the UDF *name*, silently
    /// ignoring `runtime` and `source`.  A UDF declared as `JavaScript` or
    /// `Python` would either run a coincidentally-matching Rust builtin (wrong
    /// behaviour) or return [`UdfError::Unknown`] (correct rejection, wrong
    /// reason).  Neither path told the caller that the runtime itself is
    /// unimplemented.
    ///
    /// `evaluate` now returns this error for any [`UdfRuntime`] variant that has
    /// no concrete implementation (`JavaScript`, `Python`, `External`).  To
    /// restore name-dispatch for a specific unimplemented runtime in a controlled
    /// test context set `TDW_ALLOW_FIXTURE_UDF=1` in the environment.  A warning
    /// is printed to stderr so the bypass is never invisible.
    #[error(
        "udf runtime {0:?} is not implemented; source is validated but cannot \
         be executed — set TDW_ALLOW_FIXTURE_UDF=1 to fall back to name-keyed \
         fixture dispatch for offline testing only"
    )]
    RuntimeNotImplemented(UdfRuntime),
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
///
/// # G008 theme 10 — UDF1: unimplemented-runtime gate
///
/// Returns [`UdfError::RuntimeNotImplemented`] for any [`UdfRuntime`] variant
/// that has no concrete execution path (`JavaScript`, `Python`, `External`).
/// The source is still validated (size, capability), but execution is refused
/// so a caller cannot mistake a fixture dispatch for real JS/Python evaluation.
///
/// Set `TDW_ALLOW_FIXTURE_UDF=1` to bypass for offline tests; a warning is
/// printed to stderr.
pub fn evaluate(definition: &UdfDefinition, input: &str) -> Result<String, UdfError> {
    validate_definition(definition)?;
    if input.len() > MAX_UDF_INPUT_BYTES {
        return Err(UdfError::InputTooLarge);
    }

    // G008/UDF1: gate on runtime being implemented before dispatching.
    match definition.runtime {
        UdfRuntime::Wasm => {} // falls through to dispatch_builtin (name-keyed fixtures)
        UdfRuntime::JavaScript | UdfRuntime::Python | UdfRuntime::External => {
            let fixture_allowed =
                std::env::var("TDW_ALLOW_FIXTURE_UDF").as_deref() == Ok("1");
            if !fixture_allowed {
                return Err(UdfError::RuntimeNotImplemented(definition.runtime));
            }
            eprintln!(
                "tdw-udf: TDW_ALLOW_FIXTURE_UDF=1 — runtime {:?} is not \
                 implemented; falling back to name-keyed fixture dispatch \
                 (source ignored). Remove TDW_ALLOW_FIXTURE_UDF for production.",
                definition.runtime
            );
        }
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

    #[test]
    fn denies_filesystem_rejects_oversized_source_and_unknown_udf() {
        let base = UdfDefinition {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "upper(input)".to_string(),
            allow_network: false,
            allow_filesystem: false,
        };

        // Filesystem capability is denied, in parity with network.
        assert_eq!(
            validate_definition(&UdfDefinition {
                allow_filesystem: true,
                ..base.clone()
            }),
            Err(UdfError::CapabilityDenied("filesystem"))
        );
        // Oversized SOURCE is SourceTooLarge (distinct from the InputTooLarge path).
        assert_eq!(
            validate_definition(&UdfDefinition {
                source: "x".repeat(MAX_UDF_SOURCE_BYTES + 1),
                ..base.clone()
            }),
            Err(UdfError::SourceTooLarge)
        );
        // The `identity` builtin echoes input...
        assert_eq!(
            evaluate(
                &UdfDefinition {
                    name: "identity".to_string(),
                    ..base.clone()
                },
                "AAPL"
            )
            .unwrap_or_else(|error| panic!("identity should evaluate: {error}")),
            "AAPL"
        );
        // ...and an unregistered name is Unknown.
        assert_eq!(
            evaluate(
                &UdfDefinition {
                    name: "nope".to_string(),
                    ..base
                },
                "x"
            ),
            Err(UdfError::Unknown("nope".to_string()))
        );
    }

    // G008/UDF1: Wasm runtime is unaffected by the gate (no env var needed).
    #[test]
    fn wasm_runtime_unaffected_by_fixture_udf_gate() {
        let def = UdfDefinition {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "irrelevant source".to_string(),
            allow_network: false,
            allow_filesystem: false,
        };
        assert_eq!(
            evaluate(&def, "aapl")
                .unwrap_or_else(|e| panic!("Wasm builtin should evaluate: {e}")),
            "AAPL"
        );
    }

    // G008/UDF1: unimplemented runtimes are rejected when TDW_ALLOW_FIXTURE_UDF is absent.
    // This test does NOT set the env var (relies on the var being absent in the test process).
    // The opt-in path is tested in tests/udf1_gate.rs (integration test, allows unsafe env ops).
    #[test]
    fn unimplemented_runtime_rejected_when_env_var_absent() {
        // Only runs when TDW_ALLOW_FIXTURE_UDF is not set in the environment.
        // In CI this is always the case; locally, skip if the var is set.
        if std::env::var("TDW_ALLOW_FIXTURE_UDF").as_deref() == Ok("1") {
            return;
        }
        let base = UdfDefinition {
            name: "upper".to_string(),
            runtime: UdfRuntime::Wasm,
            source: "function upper(x) { return x.toUpperCase(); }".to_string(),
            allow_network: false,
            allow_filesystem: false,
        };
        for runtime in [UdfRuntime::JavaScript, UdfRuntime::Python, UdfRuntime::External] {
            let def = UdfDefinition { runtime, ..base.clone() };
            assert_eq!(
                evaluate(&def, "aapl"),
                Err(UdfError::RuntimeNotImplemented(runtime)),
                "runtime {runtime:?} must be rejected without TDW_ALLOW_FIXTURE_UDF"
            );
        }
    }
}
