# tdw-udf — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`:

| Item | Role |
| --- | --- |
| `UdfRuntime` | Which runtime backs a UDF: `JavaScript` / `Python` / `Wasm` / `External`. |
| `UdfDefinition` | The runtime-agnostic definition + capability flags. |
| `UdfError` | The shared error enum. |
| `validate_definition` | Sandbox validation. |
| `evaluate` | `validate_definition` + builtin dispatch. |
| `MAX_UDF_SOURCE_BYTES` / `MAX_UDF_INPUT_BYTES` | Size caps (16 KiB / 64 KiB). |

## UDF runtime contract

This crate fixes the *shape* every runtime shares; the per-runtime crates own the
actual execution. A `UdfDefinition` carries:

- `name` — must be non-empty and `[A-Za-z0-9_-]` only (path-traversal safe).
- `runtime` — selects the executing crate.
- `source` — the function body; non-empty, ≤ `MAX_UDF_SOURCE_BYTES`.
- `allow_network` / `allow_filesystem` — capability requests.

## Sandbox / capability design

`validate_definition` is **deny-by-default**:

1. `name` must pass `is_udf_name` (else `InvalidDefinition("name")`).
2. `source` must be non-empty after trim (else `InvalidDefinition("source")`).
3. `source.len() > MAX_UDF_SOURCE_BYTES` → `SourceTooLarge`.
4. `allow_network == true` → `CapabilityDenied("network")`.
5. `allow_filesystem == true` → `CapabilityDenied("filesystem")`.

Today the base sandbox denies *all* network and filesystem access outright —
requesting either is a hard error, not a grant. This makes the safe path the
default; loosening a capability is a deliberate future change, not an accident.

`evaluate` additionally caps input size (`input.len() > MAX_UDF_INPUT_BYTES` →
`InputTooLarge`) before dispatch.

## Dispatch flow

```
UdfDefinition + input
   │  validate_definition  (name, source, caps, capabilities)
   │  input size cap
   ▼
 dispatch_builtin(name, input)
   "upper"    → input.to_ascii_uppercase()
   "identity" → input
   other      → UdfError::Unknown(name)
```

The builtins are a deterministic, dependency-free baseline that proves the
validate→dispatch pipeline. Real per-runtime execution (JS/Python/WASM/external)
is layered on by the sibling crates, which reuse these same definition + capability
contracts.

## Offline test design

All tests are pure unit tests over `validate_definition` / `evaluate`: no
network, no filesystem, no runtime engine. They assert the allowed path
(`upper` → `AAPL`), the capability-denied path, name rejection, empty source, and
the oversized-input cap.
