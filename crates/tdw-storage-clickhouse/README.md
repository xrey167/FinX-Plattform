# tdw-storage-clickhouse

ClickHouse [`OlapEngine`](../tdw-core/src/lib.rs) for the FinX data-warehouse
analytics tier.

## Purpose

Executes OLAP DDL/queries and builds idempotent ingest statements for the
columnar tier. Ships:

- [`ClickHouseRecordingEngine`] — always available, no network. Validates and
  records statements / echoes queries for offline tests. Also implements
  `WriteSink<T>`.
- [`ClickHouseHttpEngine`] — real reqwest HTTP backend behind the `clickhouse`
  feature (ClickHouse native HTTP interface, port 8123).

It also exposes pure helper functions (always available) used by the ingest
path:

- `build_insert_jsoneachrow(table, batch, dedup_token)` — builds an
  `INSERT … SETTINGS … FORMAT JSONEachRow` statement with dependent-MV dedup
  settings.
- `ingest_dedup_token(session_id, sequence, table)` — protocol-coordinate dedup
  token (retry-stable).
- `batch_dedup_token(session_id, table, batch)` — content-addressed dedup token
  for streaming batches with no protocol sequence.

## Engine trait

`OlapEngine`:

- `execute(ddl) -> Result<()>`
- `query_json(sql, params) -> Result<Value>`

## Default (recording) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `ClickHouseRecordingEngine` | — (always built) | none |
| Real | `ClickHouseHttpEngine` | `clickhouse` | reqwest HTTP |

Default features list is empty; `cargo test --workspace` stays offline. Enable
the real backend with `--features clickhouse`.

## Connection / env vars

```rust
// endpoint, optional user, optional password (HTTP basic auth)
let engine = ClickHouseHttpEngine::new("http://127.0.0.1:8123", None, None)?;
```

`execute` issues `POST /?query=…`; `query_json` appends `FORMAT JSON` for a
parseable response. Positional param binding is deferred (ClickHouse uses
`param_<name>` query keys, a different shape from sqlx positional binding).

The env-gated integration test (`tests/http_engine.rs`) reads
`TDW_CLICKHOUSE_TEST_URL`.

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the service OLAP engine is
`ClickHouseHttpEngine`, wired by
[`select_olap_engine`](../tdw-service-api/src/app_state.rs) from:

| Env var | Meaning |
|---|---|
| `TDW_CLICKHOUSE_URL` | HTTP endpoint (required) |
| `TDW_CLICKHOUSE_USER` | basic-auth user (optional) |
| `TDW_CLICKHOUSE_PASSWORD` | basic-auth password (optional) |

A missing URL or absent `real-clickhouse` feature fails the `live` boot closed.

## Quickstart (offline)

```rust
use serde_json::json;
use tdw_core::OlapEngine;
use tdw_storage_clickhouse::ClickHouseRecordingEngine;

# async fn run() -> tdw_core::Result<()> {
let engine = ClickHouseRecordingEngine::default();
engine.execute("create table analytics.ohlc (...) engine = MergeTree order by ts").await?;
let result = engine.query_json("select count() from analytics.ohlc", json!({})).await?;
assert_eq!(result["engine"], "clickhouse-recording");
# Ok(())
# }
```

```sh
cargo run -p tdw-storage-clickhouse --example tdw-storage-clickhouse-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and
`docs/quality/production-storage-transports.md`.
