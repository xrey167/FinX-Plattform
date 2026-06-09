# tdw-exec

Headless op execution with a read-only SQL guard: turn a protocol `OpEnvelope`
into a deterministic stream of protocol events, rejecting anything that is not a
single read-only `SELECT`.

`tdw-exec` is the minimal, side-effect-free executor used for planning and for
fuzzing the SQL boundary. It accepts a `tdw-protocol` `OpEnvelope`, validates any
embedded query, and emits `Started` + `Completed` `EventMsg`s describing what
*would* run — without touching a database.

## What it provides

- `run_headless(envelope) -> ExecRun` — infallible; emits the protocol events.
- `try_run_headless(envelope) -> Result<ExecRun>` — validates first, returning
  `ExecError` for an unsafe/mutating query.
- `validate_op(op) -> Result<()>` — the op-level guard (delegates to the SQL
  guard for `RunQuery`).
- `ExecRun` (`{ events: Vec<EventMsg> }`) and `ExecError`.

## Feature flags

None. Depends on `serde`, `serde_json`, and `tdw-protocol`.

## Quickstart

```rust
use serde_json::Value;
use tdw_exec::{run_headless, try_run_headless, ExecError};
use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};

let envelope = OpEnvelope::new(
    SessionId::new("session-1").expect("session id"),
    1,
    ActorRef { actor_id: "user".into(), kind: ActorKind::User, tenant_id: None },
    Op::RunQuery { sql: "select 1".into(), plan_id: None, cost_hint: None },
);

let run = run_headless(envelope.clone());
assert!(matches!(run.events[0], tdw_protocol::EventMsg::Started { .. }));

// A mutating query is rejected by the checked entry point.
let bad = OpEnvelope::new(
    envelope.session_id.clone(), 1,
    ActorRef { actor_id: "user".into(), kind: ActorKind::User, tenant_id: None },
    Op::RunQuery { sql: "delete from raw.orders".into(), plan_id: None, cost_hint: None },
);
assert_eq!(try_run_headless(bad), Err(ExecError::NonReadOnlySql));
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-exec --example tdw_exec_basic
```

## Read-only SQL guard

`validate_read_only_sql` rejects, with the matching `ExecError`:

- empty SQL → `EmptySql`;
- control characters, comment markers (`--`, `/*`, `*/`), or mutating keywords
  (` drop `, ` delete `, ` insert `, ` update `) → `UnsafeSqlToken`;
- more than one statement (multiple `;`, or a `;` not at the end) →
  `MultipleStatements`;
- anything not starting with `select ` (or exactly `select`) → `NonReadOnlySql`.

`run_headless` is infallible and emits events for *any* op (it does not gate);
`try_run_headless` runs `validate_op` first, so callers that need the guard use
the checked form. The crate also exposes a `#[doc(hidden)]` `__fuzz_sql_guard`
shim that the nightly fuzz target drives over arbitrary bytes — it must never
panic, only reject.

## Invariants

- `#![forbid(unsafe_code)]`.
- **Read-only by construction.** Only single `SELECT` statements pass the guard.
- **No I/O, no side effects.** Execution is "plan only" — it describes the op as
  protocol events and never runs SQL.
- **Never panics on adversarial input** (fuzz contract).
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
