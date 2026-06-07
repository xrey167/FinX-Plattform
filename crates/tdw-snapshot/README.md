# tdw-snapshot

Versioned table-snapshot store (time-travel parity layer) for the FinX
data-warehouse.

## Purpose

Records immutable, monotonically versioned snapshots of a table's row-id set, so
the system can answer "what did this table contain as of version N" and "what is
the latest version" — the parity-layer / time-travel primitive. Ships:

- [`SnapshotStore`] — always available, no network. In-memory `Vec<Snapshot>`,
  the offline default and test stand-in.
- [`PgSnapshotStore`] — Postgres-backed store behind the `postgres` feature, built
  on [`tdw-storage-postgres::PgEngine`](../tdw-storage-postgres).

## Store contract

A [`Snapshot`] is `{ table, version, created_at, row_ids }`.

| Operation | `SnapshotStore` | `PgSnapshotStore` |
|---|---|---|
| `commit(table, created_at, row_ids)` | `-> Snapshot` | `async -> Result<Snapshot>` |
| `as_of_version(table, version)` | `-> Option<&Snapshot>` | `async -> Result<Option<Snapshot>>` |
| `latest(table)` | `-> Option<&Snapshot>` | `async -> Result<Option<Snapshot>>` |

`commit` allocates the next version per table automatically (max existing + 1,
starting at 1).

## Default (in-memory) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `SnapshotStore` | — (always built) | none |
| Real | `PgSnapshotStore` | `postgres` | sqlx via `PgEngine` |

Default features list is empty; `cargo test --workspace` stays offline. The
`postgres` feature pulls `tdw-storage-postgres/postgres` (sqlx + tokio).

## Connection / env vars

`PgSnapshotStore` is constructed from a connected `PgEngine`:

```rust
let engine = tdw_storage_postgres::PgEngine::connect(&url).await?;
let store = PgSnapshotStore::new(engine);
store.ensure_schema().await?;
```

The URL comes from the caller (see
[`tdw-storage-postgres`](../tdw-storage-postgres) for the live-profile
resolution).

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the daemon's durable snapshot store is
`PgSnapshotStore` on the real `PgEngine`. The in-memory `SnapshotStore` is the
offline / `service`-profile default. The crate holds no profile switch.

## Quickstart (offline)

```rust
use tdw_snapshot::SnapshotStore;

let mut store = SnapshotStore::default();
store.commit("raw.market_data_bar", "2026-05-21T00:00:00Z", vec!["a".into()]);
store.commit("raw.market_data_bar", "2026-05-21T01:00:00Z", vec!["a".into(), "b".into()]);

assert_eq!(store.as_of_version("raw.market_data_bar", 1).map(|s| s.row_ids.len()), Some(1));
assert_eq!(store.latest("raw.market_data_bar").map(|s| s.version), Some(2));
```

```sh
cargo run -p tdw-snapshot --example tdw-snapshot-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md). Durable backend rationale is in
`docs/quality/production-storage-transports.md` (G013).
