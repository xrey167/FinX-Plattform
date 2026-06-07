# tdw-sandbox architecture

A single-module crate (`src/lib.rs`) that puts a capability gate and a
size/validation gate in front of the UDF runtimes.

## Module map

| Item | Role |
|------|------|
| `SandboxRuntime` | Trait: `runtime_name() -> &'static str`, `run(UdfRequest) -> Result<UdfResponse>`. |
| `LocalUdfSandbox` | The default in-process implementation. |
| `UdfRequest` | The request: `name`, `runtime`, `source`, `input`, `allow_network`, `allow_filesystem`, optional `wasm_limits`. |
| `WasmLimitsRequest` | Per-request override (`fuel`, `max_memory_bytes`, `max_memories`), all `Option`. |
| `UdfResponse` | `{ runtime, output }`. |
| `SandboxError` | `CapabilityDenied(&'static str)`, `InvalidRequest(&'static str)`, `Udf(String)`. |
| `validate_request` | The shared name/source/input guard. |
| `run_wasm` / `resolve_wasm_limits` | `#[cfg(feature = "udf-wasm")]` routing + limit clamping. |

## Capability model

The trust model is **deny-by-default**: a UDF gets *no* ambient authority. The
only capabilities it could request are network and filesystem, and both are
refused at the boundary in the current backend — the flags exist so the contract
is explicit and so a future backend that can grant them must do so deliberately.

`LocalUdfSandbox::run` enforces, in order:

1. `validate_request` — `name` is a safe identifier, `source` is non-empty and
   `<= MAX_UDF_SOURCE_BYTES`, `input` is `<= MAX_UDF_INPUT_BYTES`. Failures are
   `SandboxError::InvalidRequest(field)`.
2. (`udf-wasm` only) if `runtime == Wasm`, hand off to `run_wasm`, which first
   denies `allow_network` / `allow_filesystem`, then routes.
3. (default) build a `tdw_udf::UdfDefinition` and call `tdw_udf::evaluate`, which
   *also* denies the network/filesystem capabilities and dispatches the builtin
   by `name`.

Either path rejects a requested capability with
`SandboxError::CapabilityDenied("network" | "filesystem")` before user code can
observe it.

## udf-wasm routing

With the `udf-wasm` feature, `run_wasm` decides between a real module and the
fixture:

```
deny allow_network / allow_filesystem
  -> base64-decode `source`
     -> if it decodes AND starts with the wasm magic (\0asm):
            execute via WasmUdfRuntime::execute_wasm_string(module, name, input, limits)
        else:
            fixture fallback: WasmUdfRuntime::execute(minimal-header, name, input)
```

- `name` is the **exported function** to call (already validated as an identifier).
- The module travels as **base64 in `source`**, gated on the wasm magic so plain
  (non-wasm) source deterministically stays on the fixture path — this is what
  keeps the pre-feature contract intact.
- Execution runs under the resolved `WasmLimits`: fuel (≈ executed bytecode ops),
  linear-memory bytes, and a memory-count cap, with deny-by-default imports.
  Fuel exhaustion surfaces as `SandboxError::Udf(_)` (a trap, never a panic).

### `resolve_wasm_limits` (clamp-down-only)

Starts from `WasmLimits::default()` (the runtime ceiling). For each field the
caller supplied, the effective value is `provided.min(ceiling)`; absent fields
keep the ceiling. The result: a request can only **tighten** a limit, never
raise it above the built-in maximum. This makes per-request limits a safe budget
knob for untrusted UDFs instead of a DoS amplifier. Threading is proven
end-to-end by a test where a `fuel: 1` override traps a module that runs fine
under the default budget.

## Back-compat

`UdfRequest.wasm_limits` is
`#[serde(default, skip_serializing_if = "Option::is_none")]`. A legacy `udf.run`
payload with no `wasm_limits` key deserializes (the field defaults to `None`) and
re-serializes without the key; a payload that carries limits round-trips them.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Capabilities checked before dispatch** on every path.
- **Validate before run** — name/source/input limits enforced first.
- **Limits clamp down only** — never widen the runtime ceiling.
- **No panics on adversarial UDFs** — WASM traps map to `SandboxError::Udf`.
- **Serde back-compat** for `wasm_limits` (default + skip-if-none).
- **Clean-room**, and workspace clippy denies `unwrap` / `dbg!` / `todo!`.
