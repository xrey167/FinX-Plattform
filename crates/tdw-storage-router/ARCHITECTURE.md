# Architecture — tdw-storage-router

## Module map

| Path | Contents |
|---|---|
| `src/lib.rs` | `StorageRouter<T>`, `RecordingSink`, `WriteSink` impls, unit tests |

Single-module crate, no feature flags.

## Trait contract & invariants

`StorageRouter<T>` implements `tdw_core::WriteSink<T>`:

- **`write_batch`** — returns `Error::Storage("storage-router has no registered
  sinks")` when empty; otherwise awaits each child sink in registration order and
  sums their `rows_written` into a single receipt named `"storage-router"`. If any
  child errors, the error propagates immediately (fail-fast; no partial-receipt
  swallowing).
- **`health_check`** — `Degraded("…no registered sinks")` when empty; otherwise
  returns the first child's `Degraded` status, or `Healthy` if all children are
  healthy.

### Composition invariants

- The router is itself a `WriteSink<T>`, so routers nest (a router can be a child
  of another router). The summed receipt makes fan-out observable: writing 1 row
  through N sinks yields `rows_written == N`.
- Sinks are stored as `Arc<dyn WriteSink<T>>`, so the same sink instance can be
  shared (and its internal counters inspected) by the caller after registration —
  see the `router_fans_out_writes_and_sums_receipts` unit test.

`RecordingSink` is a minimal `WriteSink` that adds `batch.rows.len()` to a
`Mutex<usize>` per write and reports `Healthy`; it exists as a test double / demo
sink.

## Real-vs-stub duality design

There is no real/stub split inside this crate — it is pure routing logic. The
duality is supplied by the registered sinks: offline tests register
`RecordingSink` (or the in-memory engines from sibling crates); the live service
registers the real engines. The router code is unchanged across both, which is
the point — it isolates fan-out from backend selection.

## Env-gated integration test pattern

Not applicable. No network backend, so no double-gated `tests/` file; the unit
tests (`router_accepts_specialist_sinks`,
`empty_router_rejects_writes_and_reports_degraded`,
`router_fans_out_writes_and_sums_receipts`) run under the default offline
workspace test set.

## Migration story

None. The router owns no storage and no schema.
