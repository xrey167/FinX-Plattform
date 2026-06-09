# Architecture — tdw-snapshot

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `Snapshot`, `SnapshotStore` (in-memory), unit test |
| `src/pg_snapshot.rs` | `postgres` | `PgSnapshotStore` (built on `PgEngine`), `ensure_schema` |
| `tests/snapshot_versions.rs` | always | in-memory versioning test |
| `tests/pg_snapshot.rs` | `postgres` + env | double-gated Postgres integration test |

`src/lib.rs` re-exports `PgSnapshotStore` under `#[cfg(feature = "postgres")]`.

## Trait / store contract & invariants

### Versioning semantics (the load-bearing invariants)

- **Monotonic per-table versions.** `commit` computes the next version as
  `max(existing versions for this table) + 1`, starting at 1. Versions are dense
  and per-table independent, so two tables version on separate counters.
- **Immutable snapshots.** A committed `Snapshot` is never mutated; a new state is
  a new version. This is what makes `as_of_version` a true time-travel lookup.
- **Latest = highest version.** `latest(table)` returns the snapshot with the max
  version for that table.

A `Snapshot` captures the table's `row_ids` set plus a caller-supplied
`created_at` timestamp; it is the parity-layer record of "what rows existed at
this version".

### Durability

`SnapshotStore` is volatile (offline / `service` default). `PgSnapshotStore`
persists each snapshot to a `tdw_snapshot` table via `PgEngine`, surviving
restarts; `ensure_schema()` creates the table lazily. The version-allocation
invariant is preserved by the Postgres store so durable versions remain dense and
monotonic per table.

## Real-vs-stub duality design

Mirrors the sibling persistence crates: the in-memory store is always compiled
(offline default); `PgSnapshotStore` is opt-in behind `postgres`. Both share the
`Snapshot` value type. The in-memory store is the reference model for the
versioning rules; the Postgres store mirrors them durably.

## Env-gated integration test pattern

`tests/pg_snapshot.rs` is **double-gated**: compiled only with `--features
postgres`, runs only when the Postgres test URL is set.
`tests/snapshot_versions.rs` covers the in-memory store under the default offline
workspace test set.

## Migration story

`PgSnapshotStore::ensure_schema()` self-provisions its `tdw_snapshot` table
(`CREATE TABLE IF NOT EXISTS`), independent of the main
[`tdw-migration`](../tdw-migration) catalog. (The warehouse also defines a
`system.snapshot_version` table in the Postgres migrations for the broader parity
layer; this crate's runtime table is a separate, self-provisioning concern.)
