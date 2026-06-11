//! Integration tests for G008 theme 10 — UW2: fixture-execution gate.
//!
//! **Gate-closed path** (no env var): covered by the
//! `execute_refuses_when_env_var_absent` unit test in `lib.rs`.
//!
//! **Gate-open path** (env var set): run with:
//!
//! ```text
//! TDW_ALLOW_FIXTURE_EXECUTION=1 cargo test -p tdw-udf-wasm --test uw2_gate
//! ```
//!
//! When `TDW_ALLOW_FIXTURE_EXECUTION` is not set the tests skip gracefully.

use tdw_udf_wasm::{WasmUdfError, WasmUdfRuntime};

fn minimal_wasm() -> Vec<u8> {
    vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]
}

/// With `TDW_ALLOW_FIXTURE_EXECUTION=1`, the fixture path is reachable and
/// deterministic.
#[test]
#[cfg(not(feature = "wasmi"))]
fn execute_fixture_path_reachable_with_opt_in() {
    if std::env::var("TDW_ALLOW_FIXTURE_EXECUTION").as_deref() != Ok("1") {
        eprintln!(
            "SKIP: set TDW_ALLOW_FIXTURE_EXECUTION=1 to run the fixture opt-in test"
        );
        return;
    }
    let rt = WasmUdfRuntime::new();
    let wasm = minimal_wasm();

    let result = rt
        .execute(&wasm, "upper", "aapl")
        .unwrap_or_else(|e| panic!("fixture path must execute with opt-in: {e}"));
    assert_eq!(result, "AAPL");

    // Idempotent — same input always gives same output.
    let result2 = rt
        .execute(&wasm, "upper", "aapl")
        .unwrap_or_else(|e| panic!("idempotent run must succeed: {e}"));
    assert_eq!(result, result2);
}

/// Without `TDW_ALLOW_FIXTURE_EXECUTION`, `execute` must refuse.
///
/// Skip gracefully if the env var is set.
#[test]
#[cfg(not(feature = "wasmi"))]
fn execute_refuses_without_opt_in() {
    if std::env::var("TDW_ALLOW_FIXTURE_EXECUTION").as_deref() == Ok("1") {
        eprintln!(
            "SKIP: TDW_ALLOW_FIXTURE_EXECUTION=1 is set; gate-closed path not testable"
        );
        return;
    }
    let rt = WasmUdfRuntime::new();
    assert_eq!(
        rt.execute(&minimal_wasm(), "upper", "aapl"),
        Err(WasmUdfError::HardenedRuntimeNotCompiled),
        "execute must return HardenedRuntimeNotCompiled when wasmi is off and \
         TDW_ALLOW_FIXTURE_EXECUTION is not set"
    );
}

/// Unknown export still returns `UnknownExport` even with opt-in.
#[test]
#[cfg(not(feature = "wasmi"))]
fn unknown_export_still_rejected_with_opt_in() {
    if std::env::var("TDW_ALLOW_FIXTURE_EXECUTION").as_deref() != Ok("1") {
        eprintln!("SKIP: set TDW_ALLOW_FIXTURE_EXECUTION=1 to run this test");
        return;
    }
    let rt = WasmUdfRuntime::new();
    assert_eq!(
        rt.execute(&minimal_wasm(), "nonexistent", "x"),
        Err(WasmUdfError::UnknownExport)
    );
}
