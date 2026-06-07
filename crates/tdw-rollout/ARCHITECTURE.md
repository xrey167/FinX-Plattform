# Architecture — tdw-rollout

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `RolloutRecord`, `RolloutError`, `JsonlRollout`, `read_records`, unit tests (incl. concurrent-append) |
| `src/pg_rollout.rs` | `postgres` | `PgRollout` (generic over `RelationalEngine`), `ensure_schema` |

`src/lib.rs` re-exports `PgRollout` under `#[cfg(feature = "postgres")]`.

## Trait / store contract & invariants

### Durability & locking (the load-bearing invariants)

`JsonlRollout::append` is the crate's durability core:

1. Create any missing parent directories.
2. Open the file `create(true).read(true).append(true)`.
3. **`file.lock()`** — take an exclusive OS advisory lock (`std::fs::File::lock`)
   so concurrent appenders (threads or processes) are serialized; no interleaved
   half-written JSON lines.
4. Serialize the record as one JSON object, write a trailing `\n`.
5. **`file.sync_all()`** — fsync so the record is durably on disk before `append`
   returns. A crash after `append` returns cannot lose the record.

`read_all` takes a **shared** lock (`lock_shared`) and parses each non-blank line,
so a read is consistent against concurrent writers. The
`concurrent_appends_are_serialized_by_file_lock` unit test spawns 16 threads and
asserts all 16 records land intact — the locking invariant under contention.

This per-record fsync is deliberately stronger than
[`tdw-storage-fs`](../tdw-storage-fs)'s bulk blob writes (which do not fsync):
rollout frames are the replay source of truth, so each one is hardened
individually.

### `PgRollout`

`PgRollout` is generic over `Arc<dyn RelationalEngine>` rather than owning a
driver. It serializes each frame's payload and `execute`s an insert against the
injected engine; durability is then the engine's (Postgres WAL). `ensure_schema`
lazily creates the `tdw_rollout` table.

## Real-vs-stub duality design

Unusually for this group, **both** backends are durable: the default
`JsonlRollout` is a real local-disk log, and `PgRollout` is a real DB log. There
is no in-memory stub because a non-durable rollout would be useless. The
`postgres` feature adds only the `tdw-core` trait dependency (`RelationalEngine`);
the caller supplies the concrete engine, so the crate stays driver-free and the
default build links nothing extra.

## Env-gated integration test pattern

No double-gated `tests/` file: the always-available `JsonlRollout` is fully
covered (append/read + concurrent-append) by in-crate unit tests under the default
offline workspace test set. `PgRollout` is exercised through the cross-store
durability tests in [`tdw-session`](../tdw-session)
(`tests/g013_durable_cross_store.rs`) when the `g013-cross-store` feature + a
Postgres URL are present.

## Migration story

`PgRollout::ensure_schema()` self-provisions its `tdw_rollout` table
(`CREATE TABLE IF NOT EXISTS`), independent of the main
[`tdw-migration`](../tdw-migration) catalog. The JSONL backend needs no schema —
files are created lazily on first append.
