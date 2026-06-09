//! Real `wasmi`-backed `tdw-udf-wasm` example (feature-gated).
//!
//! Compiles a tiny WebAssembly module from WAT and runs it through the hardened
//! runtime under explicit `WasmLimits` (fuel + memory + deny-by-default
//! imports). Offline and deterministic — no network, no external artifacts.
//!
//! Requires the `wasmi` feature (declared via `required-features` in Cargo.toml):
//!
//! ```text
//! cargo run --example tdw_udf_wasm_wasmi_roundtrip -p tdw-udf-wasm --features wasmi
//! ```

use tdw_udf_wasm::{WasmLimits, WasmRuntimeError, WasmUdfRuntime};

fn wasm(text: &str) -> Vec<u8> {
    wat::parse_str(text).expect("wat should compile")
}

fn main() {
    let runtime = WasmUdfRuntime::new();
    let limits = WasmLimits::default();

    // 1) (i64) -> i64 export: double its argument.
    let double = wasm(
        r#"(module (func (export "double") (param i64) (result i64)
             local.get 0 i64.const 2 i64.mul))"#,
    );
    let out = runtime
        .execute_wasm_i64(&double, "double", 21, limits)
        .expect("double should execute under fuel");
    assert_eq!(out, 42);
    println!("double(21) = {out}");

    // 2) Fuel exhaustion: an infinite loop traps as FuelExhausted, never hangs.
    let spin = wasm(
        r#"(module (func (export "spin") (param i64) (result i64)
             (loop (br 0)) i64.const 0))"#,
    );
    let tight = WasmLimits {
        fuel: 10_000,
        ..WasmLimits::default()
    };
    assert_eq!(
        runtime.execute_wasm_i64(&spin, "spin", 0, tight),
        Err(WasmRuntimeError::FuelExhausted)
    );
    println!("runaway guest trapped as FuelExhausted, as expected");

    // 3) Deny-by-default imports: a module importing a host symbol is rejected.
    let importer = wasm(
        r#"(module (import "host" "secret" (func))
             (func (export "x") (param i64) (result i64) i64.const 0))"#,
    );
    assert_eq!(
        runtime.execute_wasm_i64(&importer, "x", 0, limits),
        Err(WasmRuntimeError::DisallowedImport(
            "host::secret".to_string()
        ))
    );
    println!("host import was denied, as expected");
}
