# tdw-storage-router

Fan-out [`WriteSink`](../tdw-core/src/lib.rs) that multiplexes one write batch to
many specialist storage sinks.

## Purpose

`tdw-storage-router` is the glue of the warehouse write path. [`StorageRouter<T>`]
holds a list of `WriteSink<T>` trait objects and, on `write_batch`, fans the
batch out to every registered sink, summing their receipts. This lets the ingest
pipeline write the same `OBBject` to (for example) a relational sink, an OLAP
sink, and a broker sink in one call.

It also ships [`RecordingSink`], a tiny always-available `WriteSink` that just
counts rows — used as a test double and as a building block in examples.

## Engine trait

`StorageRouter<T>` itself implements `tdw_core::WriteSink<T>` (it *is* a sink, so
routers can nest), exposing:

- `name() -> "storage-router"`
- `write_batch(batch)` — fan-out; errors if no sinks are registered
- `health_check()` — `Degraded` if empty, else the first non-healthy child status,
  else `Healthy`

Builder surface: `new()` / `default()`, `add_sink(Arc<dyn WriteSink<T>>)`,
`sink_count()`.

## Default vs real backend

Not applicable — the router is pure composition logic with **no backend and no
feature flag**. It pulls only `tdw-core` + `async-trait`. The "backends" are
whichever sinks the caller registers (e.g.
[`PostgresRecordingEngine`](../tdw-storage-postgres),
[`InMemoryBrokerSink`](../tdw-storage-broker), or any `PgEngine`-backed sink).

## Connection / env vars

None. Configuration is entirely programmatic via `add_sink`.

## `TDW_PROFILE=live` behavior

The router has no profile-specific behavior of its own. Its behavior changes only
through the sinks registered into it: in the `live` profile those sinks are the
real engines selected by `tdw-service-api::app_state`; offline they are the
in-memory / recording engines. The router code path is identical either way.

## Quickstart (offline)

```rust
use std::sync::Arc;
use tdw_core::{OBBject, WriteSink};
use tdw_storage_router::{RecordingSink, StorageRouter};
# use serde::{Deserialize, Serialize};
# use schemars::JsonSchema;
# #[derive(Clone, Serialize, Deserialize, JsonSchema)]
# struct Row { symbol: String }
# async fn run() -> tdw_core::Result<()> {
let mut router = StorageRouter::<Row>::new();
router.add_sink(Arc::new(RecordingSink::new("primary")));
router.add_sink(Arc::new(RecordingSink::new("replica")));

let batch = OBBject::new(vec![Row { symbol: "AAPL".into() }], "test", "rows");
let receipt = router.write_batch(&batch).await?;
assert_eq!(receipt.rows_written, 2); // 1 row x 2 sinks
# Ok(())
# }
```

```sh
cargo run -p tdw-storage-router --example tdw-storage-router-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md).
