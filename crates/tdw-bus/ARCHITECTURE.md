# Architecture — tdw-bus

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `BusEntry`, `RetentionWindow`, `EventBus` (bounded ring), unit tests |
| `src/pg_bus.rs` | `postgres` | `PgEventBus` (built on `PgEngine`), `ensure_schema` |
| `tests/bus_capacity.rs` | always | in-memory capacity/retention test |
| `tests/pg_bus.rs` | `postgres` + env | double-gated Postgres integration test |

`src/lib.rs` re-exports `PgEventBus` under `#[cfg(feature = "postgres")]`.

## Trait / store contract & invariants

### Log semantics (the load-bearing invariants)

- **Monotonic sequence from 1.** `publish` assigns a strictly increasing
  sequence; `next_sequence` never repeats, so a consumer cursor is globally
  meaningful.
- **Bounded retention.** `EventBus::new(capacity)` clamps capacity to `>= 1`.
  Once `events.len() > capacity`, the oldest entry is evicted from the front of
  the `VecDeque`. This is a *lossy* tail by design — slow consumers can fall off
  the back.
- **Gap detection.** `retention_window()` reports `oldest`/`newest` retained
  sequences. `has_retention_gap(requested)` is `true` when the requested cursor is
  older than `oldest_sequence` — i.e. the consumer asked for data that has already
  been evicted and must resync from a snapshot rather than the bus.
- **Lag.** `lag_since(last_seen)` = how many sequences have been published beyond
  the consumer's cursor (saturating), for backpressure/metrics.

`read_from(seq)` returns every retained entry with `sequence >= seq`, in order.

### Durability

`EventBus` is volatile and bounded (offline / `service` default). `PgEventBus`
persists each entry to a `tdw_bus` table via `PgEngine`, so the durable log
survives restarts; `ensure_schema()` creates the table lazily. The Postgres log's
retention policy is governed by the table (not a fixed in-memory ring).

#### Concurrency caveat — `PgEventBus` does NOT inherit the commit-order completeness guarantee

The in-memory `EventBus` assigns `sequence` under a single lock, so allocation
order **is** commit/visibility order and an advancing cursor (`cursor = max+1`)
never skips an entry. **`PgEventBus` does not share that property.** Its
`sequence` is a `BIGSERIAL` allocated by a non-transactional `nextval()`, so
allocation order is independent of COMMIT order (under `READ COMMITTED`, only
committed rows are visible). With **concurrent publishers**, a transaction that
allocated `sequence = 5` can COMMIT *after* one that allocated `6`. A consumer
that reads `[6]` in that window and advances its cursor to `7` will **never**
see `5` once it commits — silent, permanent loss for that consumer.

**Safe usage of `read_from(seq)` against `PgEventBus`:**
- Safe: re-reading from a **fixed low watermark** that never advances past an
  unconfirmed region (the in-process relay consumer does this and is unaffected).
- **Unsafe:** advancing the cursor to `max(sequence)+1` after each read (a
  replay/CDC-style consumer). Do **not** build such a consumer on `read_from`
  as-is.

A correct advancing consumer needs commit-order delivery — e.g. a watermark that
only exposes rows below the oldest in-flight transaction
(`pg_snapshot_xmin(pg_current_snapshot())`), a commit-ordered sequence stamped
inside the publishing txn, or logical-decoding (LSN) consumption. That change is
**deferred until such a consumer is actually wired**, because it must be verified
against a live concurrent Postgres (the `tests/pg_bus.rs` suite is feature- and
env-gated and is not exercised by default CI), and the naive "return only the
gapless contiguous prefix" fix is worse — it stalls forever on the permanent
gaps `BIGSERIAL` leaves whenever a publish transaction rolls back.

## Real-vs-stub duality design

Same pattern as the sibling persistence crates: the in-memory `EventBus` is
always compiled (offline default); `PgEventBus` is opt-in behind `postgres`
(which enables `tdw-storage-postgres/postgres`). Both share `BusEntry` /
`RetentionWindow`, so consumers are backend-agnostic. The in-memory ring is the
authoritative model for the semantics; the Postgres store mirrors them durably.

## Env-gated integration test pattern

`tests/pg_bus.rs` is **double-gated**: compiled only with `--features postgres`,
runs only when the Postgres test URL is set. `tests/bus_capacity.rs` covers the
in-memory ring (publish/read/retention/lag) under the default offline workspace
test set.

## Migration story

`PgEventBus::ensure_schema()` self-provisions its `tdw_bus` table
(`CREATE TABLE IF NOT EXISTS`), independent of the main
[`tdw-migration`](../tdw-migration) catalog. (The warehouse's archival event
spine — `system.event_archive` etc. — is defined separately in the Postgres
migrations.)
