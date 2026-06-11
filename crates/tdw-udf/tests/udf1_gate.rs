//! Integration tests for G008 theme 10 — UDF1: unimplemented-runtime gate.
#![allow(clippy::doc_markdown)] // gate/env-var names in backtick spans are intentional prose
//!
//! These tests verify both paths of the gate:
//! - **Gate fires** (TDW_ALLOW_FIXTURE_UDF absent): covered by the
//!   `unimplemented_runtime_rejected_when_env_var_absent` unit test in
//!   `lib.rs` (no env manipulation required — relies on the var being absent).
//! - **Gate open** (TDW_ALLOW_FIXTURE_UDF=1): run this integration test with
//!   the env var pre-set:
//!
//!   ```text
//!   TDW_ALLOW_FIXTURE_UDF=1 cargo test -p tdw-udf --test udf1_gate
//!   ```
//!
//! When `TDW_ALLOW_FIXTURE_UDF` is not set the tests below skip gracefully.

use tdw_udf::{UdfDefinition, UdfRuntime, evaluate};

/// With `TDW_ALLOW_FIXTURE_UDF=1`, unimplemented runtimes fall back to the
/// name-keyed Rust fixture (`upper` maps to the Rust builtin; source is
/// ignored).
///
/// Skip when the env var is absent (bypass not configured).
#[test]
fn unimplemented_runtime_allowed_with_opt_in() {
    if std::env::var("TDW_ALLOW_FIXTURE_UDF").as_deref() != Ok("1") {
        eprintln!("SKIP: set TDW_ALLOW_FIXTURE_UDF=1 to run the opt-in path test");
        return;
    }
    let def = UdfDefinition {
        name: "upper".to_string(),
        runtime: UdfRuntime::JavaScript,
        source: "function upper(x) { return x.toUpperCase(); }".to_string(),
        allow_network: false,
        allow_filesystem: false,
    };
    let result = evaluate(&def, "aapl")
        .unwrap_or_else(|e| panic!("fixture fallback should evaluate with opt-in: {e}"));
    assert_eq!(result, "AAPL");
}

/// Without `TDW_ALLOW_FIXTURE_UDF`, every unimplemented runtime is refused.
///
/// Skip gracefully if the env var is set (developer override active).
#[test]
fn unimplemented_runtimes_refused_without_opt_in() {
    if std::env::var("TDW_ALLOW_FIXTURE_UDF").as_deref() == Ok("1") {
        eprintln!("SKIP: TDW_ALLOW_FIXTURE_UDF=1 is set; gate-closed path not testable");
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
            Err(tdw_udf::UdfError::RuntimeNotImplemented(runtime)),
            "runtime {runtime:?} must be rejected without TDW_ALLOW_FIXTURE_UDF"
        );
    }
}
