# tdw-udf-external — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`, no dependencies:

| Item | Role |
| --- | --- |
| `ExternalUdfCommand` | `{ name, command, args, timeout_ms }`. |
| `ExternalUdfError` | The validation error enum. |
| `validate_command` | Static command-shape validation. |
| `CRATE_NAME` / `RUNTIME_NAME` / `MAX_TIMEOUT_MS` | Identity + bound constants. |

## UDF runtime contract

The crate is the **static gate** for the external (subprocess) runtime, not the
spawner. The daemon validates an `ExternalUdfCommand` here before any process is
launched.

## Sandbox design (static analysis)

`validate_command` rejects, in order:

1. Empty `name` → `EmptyName`.
2. `command` that is not a bare program name → `InvalidCommand`. A valid command
   has no path separator (`/`, `\`), no shell control char, and only
   `[A-Za-z0-9_-]`. This forces resolution through the system path / an
   allowlist rather than letting a UDF point at an arbitrary file.
3. Any `arg` containing a shell control char (`;`, `&`, `|`, backtick, or a
   control character) → `InvalidArgument`. Defends against argument-borne shell
   injection.
4. `timeout_ms == 0` or `> MAX_TIMEOUT_MS` (30 000 ms) → `InvalidTimeout`. Every
   external UDF is time-bounded; an unbounded run is rejected.

`contains_shell_control` is the shared predicate behind both the command-name and
argument checks.

## Offline test design

Pure unit tests over `validate_command`: an allowlisted command shape passes, and
a path-bearing command (`../runner`), a shell-injecting arg (`symbol=AAPL;rm`),
and an over-cap timeout are each rejected with the matching error. Nothing is
ever spawned.
