# tdw-outbox

Transactional-outbox store for the FinX event spine.

## Purpose

Implements the outbox pattern: events are appended with a monotonic sequence and
a `Pending` status, then marked `Dispatched` once they have been relayed. A
relay loop recovers un-dispatched events after a restart by reading everything
`pending_after` its last-seen sequence — giving at-least-once delivery without
losing events across a crash. Ships:

- [`InMemoryOutbox`] — always available, no network. The offline default and test
  stand-in.
- [`PgOutboxStore`] — Postgres-backed store behind the `postgres` feature, built on
  [`tdw-storage-postgres::PgEngine`](../tdw-storage-postgres).

## Store contract

Records are [`OutboxRecord`] `{ sequence, envelope: EventEnvelope<Value>, status }`
with [`OutboxStatus`] `Pending | Dispatched`.

| Operation | `InMemoryOutbox` | `PgOutboxStore` |
|---|---|---|
| `append(envelope)` | `-> u64` | `async -> Result<u64>` |
| `mark_dispatched(seq)` | `-> bool` | `async -> Result<bool>` |
| `pending_after(seq)` | `-> Vec<OutboxRecord>` | `async -> Result<Vec<OutboxRecord>>` |

The Postgres store adds `ensure_schema()` (lazy `CREATE TABLE IF NOT EXISTS`) and
`with_table(name)`.

## Default (in-memory) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `InMemoryOutbox` | — (always built) | none |
| Real | `PgOutboxStore` | `postgres` | sqlx via `PgEngine` |

Default features list is empty; `cargo test --workspace` stays offline. The
`postgres` feature pulls `tdw-storage-postgres/postgres` (sqlx + tokio).

## Connection / env vars

The Postgres store does not read env vars itself — it is constructed from an
already-connected `PgEngine`:

```rust
let engine = tdw_storage_postgres::PgEngine::connect(&url).await?;
let outbox = PgOutboxStore::new(engine);
outbox.ensure_schema().await?;
```

The `PgEngine` URL comes from the caller (see
[`tdw-storage-postgres`](../tdw-storage-postgres) for the `TDW_POSTGRES_URL` /
`TDW_DAEMON_PG_URL` / `DATABASE_URL` resolution used in the `live` profile).

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the daemon's durable stores are wired on top of
the real `PgEngine` (the same engine `select_relational_engine` connects). The
in-memory outbox is the offline/`service`-profile default. The outbox crate itself
holds no profile switch; the binary chooses `PgOutboxStore` when built with the
`postgres` feature and given a Postgres URL.

## Quickstart (offline)

```rust
use tdw_event::sample_event;
use tdw_outbox::InMemoryOutbox;

let mut outbox = InMemoryOutbox::default();
let first = outbox.append(sample_event("service"));
let second = outbox.append(sample_event("worker"));
outbox.mark_dispatched(first);

let pending = outbox.pending_after(0);
assert_eq!(pending.len(), 1);
assert_eq!(pending[0].sequence, second);
```

```sh
cargo run -p tdw-outbox --example tdw-outbox-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md). Durable backend rationale (built on
`PgEngine`) is in `docs/quality/production-storage-transports.md` (G013).
