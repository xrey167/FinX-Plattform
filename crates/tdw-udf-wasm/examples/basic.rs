//! Offline `tdw-udf-wasm` example: run the deterministic fixture interpreter
//! (`upper` / `identity`) and show the validation rejections.
//!
//! This is the DEFAULT path — no `wasmi` feature, no real engine, no network.
//! For the real `wasmi`-backed round-trip see `examples/wasmi_roundtrip.rs`
//! (`cargo run --example tdw_udf_wasm_wasmi_roundtrip -p tdw-udf-wasm --features wasmi`).
//!
//! ```text
//! cargo run --example tdw_udf_wasm_basic -p tdw-udf-wasm
//! ```

use tdw_udf_wasm::{WasmUdfError, WasmUdfModule, WasmUdfRuntime, validate_module};

fn main() {
    let runtime = WasmUdfRuntime::new();

    // Minimal valid WASM header (magic + version) passes the validation gate;
    // the fixture interpreter then dispatches on the export name.
    let wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];

    let upper = runtime
        .execute(&wasm, "upper", "aapl")
        .expect("upper should run");
    assert_eq!(upper, "AAPL");
    let identity = runtime
        .execute(&wasm, "identity", "AAPL")
        .expect("identity should run");
    assert_eq!(identity, "AAPL");
    println!("upper(aapl)    = {upper}");
    println!("identity(AAPL) = {identity}");

    // Deterministic: same input → same output.
    let again = runtime
        .execute(&wasm, "upper", "aapl")
        .expect("upper should run again");
    assert_eq!(again, upper);

    // A module descriptor validates the same way.
    let module = WasmUdfModule {
        module_name: "tdw-udf-wasm".to_string(),
        exported_function: "upper".to_string(),
        bytes: wasm.to_vec(),
    };
    validate_module(&module).expect("module descriptor should validate");

    // Rejections: bad magic and unknown export.
    assert_eq!(
        runtime.execute(b"not-wasm", "upper", "x"),
        Err(WasmUdfError::InvalidMagic)
    );
    assert_eq!(
        runtime.execute(&wasm, "nonexistent", "x"),
        Err(WasmUdfError::UnknownExport)
    );
    println!("invalid magic and unknown export are rejected, as expected");
}
