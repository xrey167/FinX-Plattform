# tdw-udf-wasm — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`:

| Item | Availability | Role |
| --- | --- | --- |
| `WasmUdfModule` / `validate_module` | always | Module descriptor + validation. |
| `WasmUdfError` | always | Fixture-path / validation error enum. |
| `WasmUdfRuntime` | always | The runtime handle (unit struct). |
| `WasmUdfRuntime::execute` | always | Deterministic fixture interpreter. |
| `WasmLimits` | always | Fuel + memory limits (inert without `wasmi`). |
| `wasm_backend` module | `wasmi` feature | Real `wasmi`-backed execution. |
| `WasmRuntimeError` | `wasmi` feature | Real-path error enum. |
| `execute_wasm_i64` / `execute_wasm_string` | `wasmi` feature | Real execution entry points. |

## UDF runtime contract

`WasmUdfRuntime` is `#[derive(Default)]` and `const fn new()`. Its public method
signatures are stable across the feature boundary: enabling `wasmi` *adds* the
hardened methods without changing the fixture `execute` signature, so routing a
daemon call to the hardened runtime is a wiring change, not an API break.

### Validation (shared)

Both paths gate on the same checks before doing work:

- non-empty bytes (`EmptyModuleBytes`),
- size ≤ `MAX_WASM_MODULE_BYTES` (4 MiB) (`ModuleTooLarge`),
- WASM magic `00 61 73 6d` (`InvalidMagic`),
- `func` is a valid export name `[A-Za-z0-9_-]` (`InvalidExportedFunction`).

## Fixture interpreter (default path)

`execute(wasm_bytes, func, arg)` validates, then `fixture_dispatch` maps export
names to pure-Rust transforms:

| Export | Transform |
| --- | --- |
| `upper` | ASCII upper-case |
| `lower` | ASCII lower-case |
| `identity` | echo |
| `len` | byte length as a string |
| other | `UnknownExport` |

Byte-deterministic: identical `(func, arg)` always yields identical output, with
no engine and no allocation of a VM. This is the offline test default.

## Hardened `wasmi` backend (`wasmi` feature)

`instantiate_guarded(wasm_bytes, func, limits)` is the shared setup for both real
entry points:

1. Run the shared validation (magic / size / export name).
2. `Config::consume_fuel(true)` → `Engine` → `Module::new` (decode/validate; a
   bad module is `Compile`).
3. **Deny-by-default imports**: if the module declares *any* import, reject with
   `DisallowedImport("{module}::{name}")` before instantiation. The `Linker` is
   empty, so the guest gets no host functions.
4. **Memory limits**: a `StoreLimits` built from `WasmLimits.max_memory_bytes` /
   `max_memories`; over-declaration fails as `MemoryLimitExceeded`.
5. **Fuel**: `store.add_fuel(limits.fuel)`; a runaway guest traps as
   `FuelExhausted`.

### `execute_wasm_i64(bytes, func, arg, limits)`

Looks up `func` typed as `(i64) -> i64` and calls it. A missing export →
`MissingExport`; an export with the wrong signature → `BadSignature`.

### `execute_wasm_string(bytes, func, input, limits)` — linear-memory ABI

The guest must export:

- `memory` — linear memory;
- `alloc(i32) -> i32` — returns a pointer to writable bytes;
- `<func>(in_ptr: i32, in_len: i32) -> i64` — reads `in_len` input bytes at
  `in_ptr`, writes output, returns a packed
  `((out_ptr as u64) << 32) | (out_len as u64)`.

Flow:

```
alloc(in_len) → in_ptr
memory.write(in_ptr, input)         (checked)
func(in_ptr, in_len) → packed i64
out_ptr = packed >> 32 ; out_len = packed & 0xffff_ffff
bounds-check out_ptr+out_len ≤ memory size
memory.read(out_ptr, out_len)       (checked)
String::from_utf8(bytes)            (UTF-8 required)
```

All guest memory access goes through wasmi's checked `Memory::read`/`write`, so a
malformed pointer/length yields `BadAbi` rather than a host panic; non-UTF-8
output is `BadAbi`. (`#![forbid(unsafe_code)]` holds across the whole crate.)

### Per-request `WasmLimits` (post-#158)

`WasmLimits { fuel, max_memory_bytes, max_memories }` is supplied **per call**,
not baked into the runtime, so the daemon can apply a different fuel/memory
budget per request. `Default` is `fuel = 1_000_000`,
`max_memory_bytes = 16 MiB`, `max_memories = 1`. The type exists unconditionally
(inert without the `wasmi` feature) so the config/policy layer compiles either
way.

### Error classification

`WasmRuntimeError` distinguishes: `InvalidExportedFunction`, `EmptyModuleBytes`,
`ModuleTooLarge`, `InvalidMagic`, `Compile`, `DisallowedImport`, `Instantiate`,
`MemoryLimitExceeded`, `MissingExport`, `BadSignature`, `FuelExhausted`, `Trap`,
`BadAbi`. Trap classification maps `OutOfFuel` → `FuelExhausted` and
memory/limit traps → `MemoryLimitExceeded`.

## Offline / cassette-test design

- **Fixture tests** (always compiled) cover `execute` for `upper`/determinism and
  the validation rejections (bad magic, oversized, unknown export). No engine, no
  network.
- **`wasmi` tests** (`#[cfg(all(test, feature = "wasmi"))]`) compile tiny guests
  from WAT with the `wat` dev-dependency and assert the full hardening matrix:
  string-ABI round-trip, distinct output, fuel exhaustion, missing `memory`,
  out-of-bounds output, non-UTF-8 output, `(i64)->i64` execution, disallowed
  import, memory-limit-at-instantiation, malformed module, missing export, wrong
  signature, and bad magic. These are the in-repo "cassettes" for the real path —
  deterministic and offline (no external WASM artifacts).
