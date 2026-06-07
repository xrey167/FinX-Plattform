# Architecture — tdw-outbox

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `OutboxStatus`, `OutboxRecord`, `InMemoryOutbox`, unit test |
| `src/pg_outbox.rs` | `postgres` | `PgOutboxStore` (built on `PgEngine`), `ensure_schema` |
| `tests/outbox_lifecycle.rs` | always | in-memory lifecycle test |
| `tests/pg_outbox.rs` | `postgres` + env | double-gated Postgres integration test |

`src/lib.rs` re-exports `PgOutboxStore` under `#[cfg(feature = "postgres")]`.

## Trait / store contract & invariants

The outbox is not a `tdw_core` engine trait; it is a typed store with two
mirrored implementations sharing the `OutboxRecord` / `OutboxStatus` value types.

### Delivery semantics (the load-bearing invariant)

- **Monotonic sequence.** `append` assigns a strictly increasing sequence
  (in-memory starts at 1). The sequence is the recovery cursor.
- **At-least-once.** An event stays `Pending` until `mark_dispatched(seq)` flips
  it to `Dispatched`. A relay that reads `pending_after(last_seen)` after a
  restart re-sees every event that was appended-but-not-dispatched, so no event
  is silently dropped across a crash. Re-delivery of an already-emitted event is
  possible (hence *at-least-once*, not exactly-once); downstream dedup (e.g. the
  ClickHouse insert-dedup token) handles the duplicate.
- **`mark_dispatched` is idempotent on the lookup**: marking an unknown sequence
  returns `false` (in-memory) / `Ok(false)` (pg) rather than erroring.

### Durability

`InMemoryOutbox` is volatile (test/`service` default). `PgOutboxStore` persists
each record to a `tdw_outbox` table via `PgEngine`, so durability is Postgres's
WAL — the outbox survives process and container restarts. `ensure_schema()`
creates the table lazily (`CREATE TABLE IF NOT EXISTS`) so no external migration
step is required for the durable stores.

## Real-vs-stub duality design

Same pattern as the storage engines: the in-memory type is always compiled (the
offline default); the Postgres store is opt-in behind `postgres`, which also
enables `tdw-storage-postgres/postgres` (sqlx + tokio). The default workspace
build links no driver. The two share the record/status types so callers are
backend-agnostic.

## Env-gated integration test pattern

`tests/pg_outbox.rs` is **double-gated**: compiled only with `--features
postgres`, runs only when the Postgres test URL is set (else early-returns). The
always-on `tests/outbox_lifecycle.rs` covers the in-memory store under the default
offline workspace test set.

## Migration story

The durable store creates its own small table via `ensure_schema()`
(`CREATE TABLE IF NOT EXISTS tdw_outbox …`), independent of the main
[`tdw-migration`](../tdw-migration) catalog. (The warehouse also defines a
`system.event_outbox` table in the Postgres migrations for the archival event
spine; `tdw-outbox`'s runtime table is a separate, self-provisioning concern.)
