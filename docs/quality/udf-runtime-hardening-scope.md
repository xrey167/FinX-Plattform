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

## Not claimed

- `LocalUdfSandbox` is not an OS process sandbox, VM boundary, seccomp profile,
  cgroup, or resource-metered Wasm engine.
- `tdw-udf-wasm` is still a deterministic fixture runtime. It is suitable for
  dispatcher proof and repeatable tests, but it is not yet real wasmi execution.
- CPU time, memory pages, fuel metering, host-call allowlists, and module cache
  eviction are open runtime-hardening work.

## Follow-up path

1. Add a real `wasmi` backend behind a feature flag.
2. Introduce explicit fuel and memory limits in the public runtime contract.
3. Add host-call allowlisting and deny-by-default imports.
4. Add timeout/fuel exhaustion tests and malformed-module corpus tests.
5. Route daemon UDF calls to the hardened runtime only when the profile enables
   it; keep the deterministic fixture path for offline tests.

## Verification

- `cargo +stable test -p tdw-sandbox -p tdw-udf -p tdw-udf-wasm`
- `cargo +stable test -p tdw-sandbox --features udf-wasm`
- `cargo +stable check -p tdw-service-api --features udf-wasm`
