# Architecture — tdw-storage-broker

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `BrokerMessage`, `InMemoryBrokerSink`, `encode_jsoneachrow`, `WriteSink` impl, unit tests |
| `src/lib.rs::rskafka_sink` | `ingest-broker` | `RskafkaBrokerSink` (rskafka `PartitionClient`) |

Both engines live in one file; the real producer is an inline
`#[cfg(feature = "ingest-broker")] mod rskafka_sink` re-exported at crate root.

## Trait contract & invariants

`tdw_core::WriteSink<T>`:

- **`write_batch`** — serializes each row to one `JSONEachRow` line via the shared
  `encode_jsoneachrow` helper. The in-memory sink pushes each `(topic, payload)`
  onto a `Mutex<Vec<BrokerMessage>>`; the rskafka sink wraps each payload in a
  Kafka `Record` and produces them to partition 0. Both return a `WriteReceipt`
  whose `rows_written` equals the row count.
- An empty batch produces nothing and returns `rows_written == 0`.
- **`health_check`** — returns `Healthy`.

### Framing invariant (the load-bearing contract)

One row == one `JSONEachRow` message == one ClickHouse row. Both sinks share
`encode_jsoneachrow`, so the recording sink's captured payloads are byte-identical
to what the real producer would emit, and the same ClickHouse `Kafka` table engine
(`kafka_format = 'JSONEachRow'`) consumes either path. The Kafka record-level
timestamp is intentionally pinned to the UNIX epoch — the event time lives in the
payload (`ts`), which is what the consumer reads — so the build stays pure (no
chrono `clock` feature) and deterministic.

## Real-vs-stub duality design

Mirrors `tdw-storage-clickhouse`: the in-memory recording sink is always
compiled and is the offline default; the real `rskafka` producer is opt-in
behind `ingest-broker`. `default-features = false` on `rskafka` drops the C
compression `-sys` crates so the real sink builds with no native toolchain (e.g.
on a bare Windows host) and uses only the rustls transport.

## Env-gated integration test pattern

This crate has no `tests/` integration file; the always-available
`InMemoryBrokerSink` is fully covered by in-crate unit tests
(`records_one_jsoneachrow_message_per_row`, `empty_batch_records_nothing`) under
the default offline `cargo test --workspace`. A live Kafka smoke is exercised
through the broader compose stack rather than a per-crate double-gated test.

## Migration story

No schema of its own. The consuming ClickHouse `Kafka` table engine and its
target tables are defined in [`tdw-migration`](../tdw-migration)
(`migrations/clickhouse/20260528_0006_kafka_ingest.sql`).
