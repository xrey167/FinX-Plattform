# tdw-rollout

Durable rollout / replay-frame recorder for the FinX event spine.

## Purpose

Appends [`ReplayFrame`](../tdw-protocol)s to a durable log so a session's event
stream can be replayed deterministically (rollout / audit / recovery). Ships:

- [`JsonlRollout`] — always available, the default. A crash-safe, append-only
  JSONL file on local disk, serialized across writers by an OS file lock and
  fsynced per record.
- [`PgRollout`] — Postgres-backed sink behind the `postgres` feature, generic over
  any `tdw_core::RelationalEngine` (the concrete engine, e.g.
  [`PgEngine`](../tdw-storage-postgres), is supplied by the caller).

## Store contract

A [`RolloutRecord`] is `{ recorded_at, frame: ReplayFrame }`.

| Operation | `JsonlRollout` | `PgRollout` |
|---|---|---|
| `append(record)` | `-> Result<()>` | `async -> Result<()>` |
| `read_all()` | `-> Result<Vec<RolloutRecord>>` | `async -> Result<Vec<RolloutRecord>>` |

`PgRollout` adds `ensure_schema()` and `with_table(name)`.

## Default (local-fs JSONL) vs real backend

| | Type | Feature | Backend |
|---|---|---|---|
| Default | `JsonlRollout` | — (always built) | local disk (JSONL + flock + fsync) |
| Real (DB) | `PgRollout` | `postgres` | caller-supplied `RelationalEngine` |

The default is itself durable (disk), so there is no "in-memory stub" here — the
two implementations are *two durable backends* (file vs Postgres). The `postgres`
feature only pulls `tdw-core` for the `RelationalEngine` trait; **no driver dep
lands in this crate** — the caller injects the engine.

## Connection / env vars

`JsonlRollout` takes a file path:

```rust
let rollout = JsonlRollout::new("/var/lib/tdw/rollout/session-1.jsonl");
```

`PgRollout` takes an `Arc<dyn RelationalEngine>` (constructed/connected by the
caller — see [`tdw-storage-postgres`](../tdw-storage-postgres) for URL
resolution):

```rust
let engine: Arc<dyn RelationalEngine> = Arc::new(pg_engine);
let rollout = PgRollout::new(engine);
rollout.ensure_schema().await?;
```

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) a deployment can route rollout to Postgres via
`PgRollout` over the live `PgEngine`. The local-fs `JsonlRollout` is the default
for the `service` profile and for purely local recording. The crate holds no
profile switch; the backend is chosen by the caller's construction.

## Quickstart (offline)

```rust
use tdw_protocol::{EventMsg, OpId, ReplayFrame, SessionId};
use tdw_rollout::{JsonlRollout, RolloutRecord};

# fn run() -> tdw_rollout::Result<()> {
let rollout = JsonlRollout::new(std::env::temp_dir().join("tdw-rollout-demo.jsonl"));
let record = RolloutRecord {
    recorded_at: "2026-05-22T00:00:00Z".to_string(),
    frame: ReplayFrame {
        session_id: SessionId::new("session-1").expect("session id"),
        sequence: 1,
        event: EventMsg::Started { op_id: OpId::generated() },
    },
};
rollout.append(&record)?;
let all = rollout.read_all()?;
assert_eq!(all.len(), 1);
# Ok(())
# }
```

```sh
cargo run -p tdw-rollout --example tdw-rollout-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md). Durable backend rationale is in
`docs/quality/production-storage-transports.md` (G013).
