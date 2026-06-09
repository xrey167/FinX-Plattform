# tdw-bus

Append-only event bus (replayable log) for the FinX event spine.

## Purpose

A bounded, sequence-numbered event log that downstream consumers tail by
sequence. Publishers append envelopes; consumers `read_from(seq)` to replay
everything at or after a cursor, and use the retention helpers to detect when they
have fallen behind the bus's bounded window. Ships:

- [`EventBus`] — always available, no network. In-memory `VecDeque` ring with a
  capacity bound. The offline default and test stand-in.
- [`PgEventBus`] — Postgres-backed log behind the `postgres` feature, built on
  [`tdw-storage-postgres::PgEngine`](../tdw-storage-postgres).

## Store contract

Entries are [`BusEntry`] `{ sequence, envelope: EventEnvelope<Value> }`.
[`RetentionWindow`] `{ oldest_sequence, newest_sequence }` reports the currently
retained span.

| Operation | `EventBus` | `PgEventBus` |
|---|---|---|
| `publish(envelope)` | `-> u64` | `async -> Result<u64>` |
| `read_from(seq)` | `-> Vec<BusEntry>` | `async -> Result<Vec<BusEntry>>` |
| `retention_window()` | `-> Option<RetentionWindow>` | `async -> Result<Option<RetentionWindow>>` |
| `has_retention_gap(seq)` | `-> bool` | — |
| `lag_since(seq)` | `-> u64` | — |

## Default (in-memory) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `EventBus` | — (always built) | none |
| Real | `PgEventBus` | `postgres` | sqlx via `PgEngine` |

Default features list is empty; `cargo test --workspace` stays offline. The
`postgres` feature pulls `tdw-storage-postgres/postgres` (sqlx + tokio).

## Connection / env vars

`PgEventBus` is constructed from a connected `PgEngine`:

```rust
let engine = tdw_storage_postgres::PgEngine::connect(&url).await?;
let bus = PgEventBus::new(engine);
bus.ensure_schema().await?;
```

The URL comes from the caller (see [`tdw-storage-postgres`](../tdw-storage-postgres)
for `TDW_POSTGRES_URL` / `TDW_DAEMON_PG_URL` / `DATABASE_URL` resolution).

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the daemon's durable event log is `PgEventBus`
on top of the real `PgEngine`. The in-memory `EventBus` is the offline /
`service`-profile default. The crate holds no profile switch; the binary picks
`PgEventBus` when built with `postgres` and given a URL.

## Quickstart (offline)

```rust
use tdw_bus::EventBus;
use tdw_event::sample_event;

let mut bus = EventBus::new(4); // bounded ring, capacity 4
let first = bus.publish(sample_event("service"));
let second = bus.publish(sample_event("worker"));

let entries = bus.read_from(first);
assert_eq!(entries.len(), 2);
assert_eq!(bus.lag_since(first), second - first);
```

```sh
cargo run -p tdw-bus --example tdw-bus-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md). Durable backend rationale is in
`docs/quality/production-storage-transports.md` (G013).
