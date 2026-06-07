# tdw-storage-broker

Kafka broker ingest [`WriteSink`](../tdw-core/src/lib.rs) for the FinX
data-warehouse streaming-ingest path.

## Purpose

Produces one `JSONEachRow` message per row to a Kafka topic, so the canonical
downstream consumer — a ClickHouse `Kafka` table engine reading
`kafka_format = 'JSONEachRow'` (see
`migrations/clickhouse/20260528_0006_kafka_ingest.sql`) — turns one message into
one ClickHouse row. Ships:

- [`InMemoryBrokerSink`] — always available, no network. Serializes each row to a
  `JSONEachRow` line and records the produced `(topic, payload)` messages in
  memory so offline tests can assert what *would* be produced.
- [`RskafkaBrokerSink`] — real Kafka producer behind the `ingest-broker` feature,
  built on the pure-Rust `rskafka` client (no librdkafka / C deps). Produces the
  same `JSONEachRow` framing to a live topic.

## Engine trait

Both types implement `tdw_core::WriteSink<T>` for any `T: DataModel`:

- `name() -> &'static str`
- `write_batch(batch) -> Result<WriteReceipt>`
- `health_check() -> Result<HealthStatus>`

This is the same sink trait used by the other warehouse sinks, so a broker sink
can sit behind a [`StorageRouter`](../tdw-storage-router) alongside relational /
OLAP sinks.

## Default (recording) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `InMemoryBrokerSink` | — (always built) | none |
| Real | `RskafkaBrokerSink` | `ingest-broker` | rskafka (rustls) |

Default features list is empty; `cargo test --workspace` stays offline and pulls
no Kafka dependency. Enable the real producer with `--features ingest-broker`.

## Connection / configuration

```rust
// recording sink: just a topic name
let sink = InMemoryBrokerSink::new("tdw.tick");

// real producer: comma-separated bootstrap brokers + topic
let sink = RskafkaBrokerSink::connect("127.0.0.1:9092", "tdw.tick").await?;
```

`RskafkaBrokerSink` is built with `default-features = false` to drop the
C-backed compression codecs (lz4/zstd/snap) and use only the rustls transport,
so it builds on a host with no native toolchain.

## `TDW_PROFILE=live` behavior

The broker sink is an optional, opt-in ingest target rather than one of the five
profile-selected engines in `tdw-service-api::app_state`. There is no
profile-driven auto-wiring of the broker sink; a deployment that wants Kafka
ingest builds with `--features ingest-broker` and constructs
`RskafkaBrokerSink::connect(...)` explicitly. The default ingest path remains the
relational / OLAP engines selected by profile.

## Quickstart (offline)

```rust
use tdw_core::{OBBject, WriteSink};
use tdw_storage_broker::InMemoryBrokerSink;
# use serde::{Deserialize, Serialize};
# use schemars::JsonSchema;
# #[derive(Clone, Serialize, Deserialize, JsonSchema)]
# struct Tick { symbol: String }
# async fn run() -> tdw_core::Result<()> {
let sink = InMemoryBrokerSink::new("tdw.tick");
let batch = OBBject::new(vec![Tick { symbol: "AAPL".into() }], "ws", "equity_ticks");
let receipt = sink.write_batch(&batch).await?;
assert_eq!(receipt.rows_written, 1);
assert_eq!(sink.messages()?.len(), 1);
# Ok(())
# }
```

```sh
cargo run -p tdw-storage-broker --example tdw-storage-broker-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and
`docs/quality/production-storage-transports.md`.
