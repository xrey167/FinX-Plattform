# tdw-sandbox

The capability-gated execution boundary for user-defined functions (UDFs).

`tdw-sandbox` wraps the UDF runtimes behind one `SandboxRuntime` trait and one
request type (`UdfRequest`). Every request is validated and capability-checked
*before* any code runs, so the caller's `allow_network` / `allow_filesystem`
flags and size limits are enforced at the boundary rather than inside each
runtime.

## What it provides

- `SandboxRuntime` — the trait every sandbox backend implements
  (`runtime_name`, `run`).
- `LocalUdfSandbox` — the default in-process backend.
- `UdfRequest` — name, runtime, source, input, capability flags, and optional
  per-request `WasmLimits` override.
- `WasmLimitsRequest` — a per-request *tightening* override for the WASM runtime.
- `UdfResponse`, `SandboxError`.

## Feature flags

| Feature     | Default | Effect |
|-------------|---------|--------|
| (none)      | —       | `UdfRuntime::Wasm` requests are dispatched to the `tdw-udf` built-in fixture interpreter, which deterministically handles the `upper` and `identity` UDFs (by `name`). No real WASM is executed. |
| `udf-wasm`  | off     | Pulls in `tdw-udf-wasm` (with its `wasmi` backend) and `base64`. `LocalUdfSandbox` then routes `UdfRuntime::Wasm` requests carrying a **base64-encoded WASM module in `source`** (detected by the `\0asm` magic) through the hardened `wasmi` string ABI under `WasmLimits` (fuel cap, memory cap, deny-by-default imports). Non-WASM `source` still falls back to the fixture interpreter, preserving the prior contract. |

## Quickstart (default build, no features)

```rust
use tdw_sandbox::{LocalUdfSandbox, SandboxRuntime, UdfRequest};
use tdw_udf::UdfRuntime;

let response = LocalUdfSandbox.run(UdfRequest {
    name: "upper".to_string(),       // the fixture UDF to run
    runtime: UdfRuntime::Wasm,
    source: "upper(input)".to_string(), // non-empty; treated as opaque by the fixture
    input: "aapl".to_string(),
    allow_network: false,
    allow_filesystem: false,
    wasm_limits: None,
}).expect("udf runs");

assert_eq!(response.output, "AAPL");
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-sandbox --example tdw_sandbox_basic
```

## Capability model

Capabilities are **deny-by-default and checked first**:

- `allow_network: true` → `SandboxError::CapabilityDenied("network")` *before any
  UDF runs* (including on the `udf-wasm` path).
- `allow_filesystem: true` → `SandboxError::CapabilityDenied("filesystem")`.
- `validate_request` then enforces: a valid UDF `name` (`[A-Za-z0-9_-]+`,
  non-empty), non-empty `source`, `source.len() <= MAX_UDF_SOURCE_BYTES`,
  `input.len() <= MAX_UDF_INPUT_BYTES`.

`WasmLimitsRequest` can only ever **tighten** a limit: each provided field is
clamped down to the runtime ceiling (`WasmLimits::default()`), so a caller can
shrink an untrusted UDF's fuel/memory budget but never raise it above the
built-in maximum (which would be a DoS lever).

## Invariants

- `#![forbid(unsafe_code)]`.
- Capability checks (`network`, `filesystem`) run **before** dispatch on every
  path.
- Per-request WASM limits are clamp-down-only (never widen the ceiling).
- `wasm_limits` is `serde(default, skip_serializing_if = "Option::is_none")`, so
  existing `udf.run` payloads deserialize and re-serialize unchanged.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the dispatch/routing detail.
