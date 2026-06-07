# tdw-udf

Core user-defined-function (UDF) contract and sandbox validation for the TDW
daemon.

## Purpose

`tdw-udf` defines the runtime-agnostic UDF definition type, the shared sandbox
validation rules (deny-by-default capabilities + size caps), and a small builtin
dispatcher. Per-runtime crates (`tdw-udf-js`, `tdw-udf-python`, `tdw-udf-wasm`,
`tdw-udf-external`) build on these contracts.

Core surface:

- [`UdfDefinition`] — `{ name, runtime, source, allow_network, allow_filesystem }`.
- [`UdfRuntime`] — `JavaScript` / `Python` / `Wasm` / `External`.
- [`UdfError`] — `CapabilityDenied`, `InvalidDefinition`, `SourceTooLarge`,
  `InputTooLarge`, `Unknown`.
- [`validate_definition`] — name/source hygiene + capability + size checks.
- [`evaluate`] — validate then dispatch a builtin (`upper`, `identity`).
- Caps: `MAX_UDF_SOURCE_BYTES = 16 KiB`, `MAX_UDF_INPUT_BYTES = 64 KiB`.

## Feature flags

None. Dependencies are `serde` and `thiserror` only.

## Environment variables

None.

## Quickstart

```rust
use tdw_udf::{UdfDefinition, UdfRuntime, evaluate};

let definition = UdfDefinition {
    name: "upper".to_string(),
    runtime: UdfRuntime::Wasm,
    source: "(input) => input.toUpperCase()".to_string(),
    allow_network: false,
    allow_filesystem: false,
};
assert_eq!(evaluate(&definition, "aapl")?, "AAPL");
# Ok::<(), tdw_udf::UdfError>(())
```

`evaluate` returns `CapabilityDenied` if `allow_network`/`allow_filesystem` is
set, `SourceTooLarge`/`InputTooLarge` past the caps, and `Unknown` for a name
that is not a builtin.

## Example

```text
cargo run --example tdw_udf_basic -p tdw-udf
```

`examples/basic.rs` validates a definition and evaluates the `upper`/`identity`
builtins, then shows the capability-denied path — no network, no filesystem.
