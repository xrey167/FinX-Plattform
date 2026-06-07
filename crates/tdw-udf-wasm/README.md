# tdw-udf-wasm

WebAssembly UDF runtime for the TDW daemon — a deterministic offline fixture
path plus a hardened, real `wasmi`-backed path behind a feature flag.

## Purpose

Two execution paths share one crate, both `#![forbid(unsafe_code)]`:

1. **Default — deterministic fixture interpreter.** [`WasmUdfRuntime::execute`]
   validates the WASM magic bytes + size, then maps a small set of export names
   (`upper`, `lower`, `identity`, `len`) to pure-Rust transforms. Byte-deterministic
   and engine-free, so it is the offline test default and needs no extra deps.
2. **Hardened — real `wasmi` backend** (`wasmi` feature). Runs a real WebAssembly
   module through the pure-Rust `wasmi` interpreter under explicit
   [`WasmLimits`]: [`WasmUdfRuntime::execute_wasm_i64`] (an `(i64) -> i64`
   export) and [`WasmUdfRuntime::execute_wasm_string`] (a string-in/string-out
   export over a linear-memory ABI).

## Feature flags

| Feature | Effect |
| --- | --- |
| `wasmi` | Compiles the real `wasmi`-backed runtime (`execute_wasm_i64`, `execute_wasm_string`, `WasmRuntimeError`) with fuel metering, memory limits, and deny-by-default imports. Pulls in the `wasmi` crate. **Off by default** so the deterministic fixture path stays the offline test default. |

`WasmLimits` is part of the stable contract **regardless** of the feature, so
config layers can construct limits unconditionally (they are inert when the
feature is off).

## Environment variables

None. Resource limits are passed explicitly via [`WasmLimits`], not read from the
environment.

## Quickstart

Fixture path (default, no feature):

```rust
use tdw_udf_wasm::WasmUdfRuntime;

let runtime = WasmUdfRuntime::new();
// Minimal valid WASM header (magic + version) satisfies the validation gate.
let wasm = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
assert_eq!(runtime.execute(&wasm, "upper", "aapl")?, "AAPL");
# Ok::<(), tdw_udf_wasm::WasmUdfError>(())
```

Hardened path (`--features wasmi`):

```rust,ignore
use tdw_udf_wasm::{WasmLimits, WasmUdfRuntime};

let runtime = WasmUdfRuntime::new();
let out = runtime.execute_wasm_i64(&module_bytes, "double", 21, WasmLimits::default())?;
assert_eq!(out, 42);
```

## Examples

Fixture example (default features):

```text
cargo run --example tdw_udf_wasm_basic -p tdw-udf-wasm
```

Real `wasmi` example (requires the feature — declared with
`required-features = ["wasmi"]`):

```text
cargo run --example tdw_udf_wasm_wasmi_roundtrip -p tdw-udf-wasm --features wasmi
```

## Gates

The `wasmi` path has dedicated CI coverage; run it locally with:

```text
cargo clippy -p tdw-udf-wasm --features wasmi --all-targets -- -D warnings
cargo test  -p tdw-udf-wasm --features wasmi
```
