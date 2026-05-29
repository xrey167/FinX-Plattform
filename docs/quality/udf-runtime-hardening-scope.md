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

## Not claimed

- `LocalUdfSandbox` is not an OS process sandbox, VM boundary, seccomp profile,
  or cgroup. The `wasmi` backend is an in-process, resource-metered interpreter,
  not an OS-level isolation boundary.
- The hardened runtime currently exposes an `(i64) -> i64` calling convention;
  a linear-memory string/bytes ABI and module cache are not yet implemented.

## Follow-up path

1. ✅ Real `wasmi` backend behind the `wasmi` feature flag.
2. ✅ Explicit fuel and memory limits in the public runtime contract
   (`WasmLimits`, part of the stable contract even with the feature off).
3. ✅ Deny-by-default host imports (empty `Linker`).
4. ✅ Fuel-exhaustion, memory-limit, malformed-module, disallowed-import,
   missing-export, and bad-signature tests (`wasmi_tests`, gated on the feature).
5. Route daemon UDF calls to the hardened runtime when a profile enables it;
   keep the deterministic fixture path for offline tests. **(remaining)** This
   needs a string/bytes ABI over linear memory plus profile-driven selection in
   `tdw-sandbox`/`tdw-service-api`.

## Verification

- `cargo +stable test -p tdw-sandbox -p tdw-udf -p tdw-udf-wasm`
- `cargo +stable test -p tdw-udf-wasm --features wasmi`
- `cargo +stable clippy -p tdw-udf-wasm --features wasmi --all-targets -- -D warnings`
- `cargo +stable test -p tdw-sandbox --features udf-wasm`
- `cargo +stable check -p tdw-service-api --features udf-wasm`
