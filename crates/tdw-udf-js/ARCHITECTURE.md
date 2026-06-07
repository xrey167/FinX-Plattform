# tdw-udf-js — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`, no dependencies:

| Item | Role |
| --- | --- |
| `JavaScriptUdfModule` | `{ module_name, entrypoint, source }`. |
| `JavaScriptUdfError` | The validation error enum. |
| `validate_module` | Static sandbox validation. |
| `CRATE_NAME` / `RUNTIME_NAME` | Identity constants (`RUNTIME_NAME = "javascript"`). |

## UDF runtime contract

This crate is the **static gate** for the JavaScript runtime, not the runtime
itself. The daemon validates a `JavaScriptUdfModule` here before any engine sees
it; a real JS engine integration would consume already-validated modules.

## Sandbox design (static analysis)

`validate_module` rejects, in order:

1. Empty `module_name` → `EmptyModuleName`.
2. `entrypoint` that is not a dotted identifier path → `InvalidEntrypoint`
   (each `.`-separated segment must be non-empty and `[A-Za-z0-9_$]` only).
3. Empty `source` → `EmptySource`.
4. `source` containing `import(` → `DynamicImportDenied` (no dynamic module
   loading — closes an exfiltration/escape vector).
5. `source` containing `fetch(` or `XMLHttpRequest` → `NetworkAccessDenied`.

This is deliberately a conservative *substring* check: it errs toward rejecting
anything that could reach the network or load arbitrary modules. It is a
pre-execution filter, not a full parser, so it favours false-positives over
letting an unsafe pattern through.

## Offline test design

Pure unit tests over `validate_module`: the safe-module contract passes, and the
network (`fetch(`) and dynamic-import (`import(`) patterns are rejected. No
engine, no network, no filesystem.
