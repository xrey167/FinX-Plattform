# UDF Runtime Hardening Scope

Date: 2026-05-28

This cycle proves that UDF dispatch is policy-gated and bounded before it enters
the local sandbox. It does not claim full language-runtime isolation.

## Enforced now

- `tdw-service-api` runs UDF work through the same policy guard as the daemon
  request path before calling `LocalUdfSandbox`.
- `tdw-sandbox` rejects invalid UDF names, empty or oversized source text,
  oversized input, and denied network/filesystem capability flags before runtime
  dispatch.
- `tdw-udf` enforces `MAX_UDF_SOURCE_BYTES` and `MAX_UDF_INPUT_BYTES` for the
  built-in deterministic dispatcher.
- `tdw-udf-wasm` validates module magic bytes, export names, module byte limits,
  and unknown exports before fixture dispatch.
- `tdw-udf-wasm` ships a real, resource-bounded `wasmi` backend behind the
  `wasmi` feature (`WasmUdfRuntime::execute_wasm_i64`): fuel metering bounds CPU
  (runaway guests trap as `FuelExhausted`), `WasmLimits` caps linear memory
  (over-allocation fails as `MemoryLimitExceeded`), and an empty `Linker` denies
  all host imports by default (modules importing any symbol are rejected before
  instantiation). The deterministic fixture path remains the default for offline
  tests.
- `WasmUdfRuntime::execute_wasm_string` adds a string-in/string-out linear-memory
  ABI under the same fuel/memory/deny-imports guarantees: the guest exports
  `memory` + `alloc(i32)->i32` + `<func>(in_ptr,in_len)->i64` (returning packed
  `(out_ptr,out_len)`). All guest memory access uses wasmi's checked
  `Memory::read`/`write`, so a bad pointer/length or non-UTF-8 output yields
  `BadAbi` rather than a host panic (`#![forbid(unsafe_code)]` is preserved).

## Not claimed

- `LocalUdfSandbox` is not an OS process sandbox, VM boundary, seccomp profile,
  or cgroup. The `wasmi` backend is an in-process, resource-metered interpreter,
  not an OS-level isolation boundary.
- A module cache is not implemented (each call re-compiles).
- Per-request `WasmLimits` overrides are not exposed; the sandbox routing uses
  `WasmLimits::default()`.

## Follow-up path

1. ✅ Real `wasmi` backend behind the `wasmi` feature flag.
2. ✅ Explicit fuel and memory limits in the public runtime contract
   (`WasmLimits`, part of the stable contract even with the feature off).
3. ✅ Deny-by-default host imports (empty `Linker`).
4. ✅ Fuel-exhaustion, memory-limit, malformed-module, disallowed-import,
   missing-export, bad-signature, and string-ABI (echo / distinct-output /
   out-of-bounds / non-UTF-8) tests (`wasmi_tests`, gated on the feature).
5. ✅ String/bytes ABI over linear memory (`execute_wasm_string`) **and** routing:
   with `tdw-sandbox`'s `udf-wasm` feature (which now enables
   `tdw-udf-wasm/wasmi`), `LocalUdfSandbox` runs a `UdfRuntime::Wasm` request
   through `execute_wasm_string` when `UdfRequest.source` base64-decodes to a
   real wasm module (magic `\0asm`); otherwise it falls back to the
   deterministic fixture. Default `cargo test --workspace` (no feature) keeps the
   `tdw-udf` built-in dispatcher. **Step #5 is complete.**

UDF runtime hardening (this scope) is now fully delivered. Remaining
enhancements are out of scope: per-request `WasmLimits`, a module cache, and a
`wasm_bytes` struct field (base64-in-`source` was chosen instead).

## Verification

- `cargo +stable test -p tdw-sandbox -p tdw-udf -p tdw-udf-wasm`
- `cargo +stable test -p tdw-udf-wasm --features wasmi`
- `cargo +stable clippy -p tdw-udf-wasm --features wasmi --all-targets -- -D warnings`
- `cargo +stable test -p tdw-sandbox --features udf-wasm`
- `cargo +stable check -p tdw-service-api --features udf-wasm`
