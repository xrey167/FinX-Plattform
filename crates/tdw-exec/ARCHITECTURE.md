# tdw-exec architecture

A single-module crate (`src/lib.rs`): a headless op executor and the read-only
SQL guard that protects it.

## Module map

| Item | Role |
|------|------|
| `ExecRun` | `{ events: Vec<EventMsg> }` — the produced protocol events. |
| `ExecError` | `EmptySql` / `MultipleStatements` / `UnsafeSqlToken` / `NonReadOnlySql` (impls `Display` + `std::error::Error`). |
| `run_headless(envelope)` | Infallible; emits `Started` + `Completed`. |
| `try_run_headless(envelope)` | `validate_op` then `run_headless`. |
| `validate_op(op)` | Op-level guard; routes `RunQuery` to the SQL guard. |
| `__fuzz_sql_guard(&[u8])` | `#[doc(hidden)]` fuzz shim (never panics). |
| `validate_read_only_sql` (private) | The SQL guard itself. |

## Core contract

The crate's purpose is to be the *safe, deterministic, no-I/O* execution path:
given a `tdw-protocol` `OpEnvelope`, describe what would happen as protocol
events. It deliberately runs nothing.

### `run_headless` vs `try_run_headless`

`run_headless` always emits a `Started { op_id }` followed by a `Completed`:

- for `Op::RunQuery { sql, .. }` the completion summary is `"query planned"` with
  the SQL echoed in the result;
- for any other op, `"op accepted"` with the op echoed.

It does **not** validate — it is the raw projection. `try_run_headless` is the
guarded front door: it calls `validate_op` first and only then `run_headless`, so
an unsafe query is rejected before any event is produced.

### The SQL guard

`validate_read_only_sql` is the security-critical core, applied to `RunQuery`:

1. reject empty SQL (`EmptySql`);
2. reject any control character (`UnsafeSqlToken`);
3. count `;`: more than one, or one not at the end, is `MultipleStatements`
   (defeats statement stacking);
4. strip a single trailing `;`, lowercase, and require the statement to start
   with `select ` (or be exactly `select`), else `NonReadOnlySql`;
5. reject comment markers (`--`, `/*`, `*/`) and mutating keywords (` drop `,
   ` delete `, ` insert `, ` update `) as `UnsafeSqlToken`.

The ordering matters: structural checks (control chars, statement count) run
before the keyword scan, and the read-only prefix check gates everything that
follows.

### Fuzz shim

`__fuzz_sql_guard` feeds arbitrary bytes (lossy-UTF-8) into the guard as a query.
Its contract is *never panic* — adversarial input must surface as an `ExecError`,
not a crash. It is shared with the nightly cargo-fuzz target and the stable
corpus-replay harness, so the guard's panic-freedom is continuously exercised.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Read-only by construction** — only single `SELECT` statements pass.
- **No I/O / no side effects** — "plan only" execution.
- **Panic-free on adversarial input** (fuzz contract).
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
