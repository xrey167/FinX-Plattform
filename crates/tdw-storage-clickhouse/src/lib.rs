#![forbid(unsafe_code)]

#[cfg(feature = "clickhouse")]
pub mod http_engine;

#[cfg(feature = "clickhouse")]
pub use http_engine::ClickHouseHttpEngine;

use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;
use tdw_core::{
    DataModel, Error, HealthStatus, OBBject, OlapEngine, Result, WriteReceipt, WriteSink,
};

#[derive(Debug, Default)]
pub struct ClickHouseRecordingEngine {
    statements: Mutex<Vec<String>>,
    rows_written: Mutex<usize>,
}

impl ClickHouseRecordingEngine {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn statements(&self) -> Result<Vec<String>> {
        self.statements
            .lock()
            .map(|statements| statements.clone())
            .map_err(|error| Error::Storage(error.to_string()))
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn rows_written(&self) -> Result<usize> {
        self.rows_written
            .lock()
            .map(|rows| *rows)
            .map_err(|error| Error::Storage(error.to_string()))
    }
}

#[async_trait]
impl OlapEngine for ClickHouseRecordingEngine {
    async fn execute(&self, ddl: &str) -> Result<()> {
        validate_sql(ddl)?;
        self.statements
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?
            .push(ddl.to_string());
        Ok(())
    }

    async fn query_json(&self, sql: &str, params: Value) -> Result<Value> {
        validate_sql(sql)?;
        Ok(serde_json::json!({
            "engine": "clickhouse-recording",
            "sql": sql,
            "params": params
        }))
    }
}

fn validate_sql(sql: &str) -> Result<()> {
    if sql.trim().is_empty() {
        return Err(Error::Storage(
            "clickhouse sql must not be empty".to_string(),
        ));
    }
    Ok(())
}

/// Build an idempotent ClickHouse `INSERT … FORMAT JSONEachRow` statement for a
/// batch of rows.
///
/// The statement carries per-query `SETTINGS`:
/// - `async_insert=1` — server-side buffering so many small ingests do not each
///   create a part (documented flush thresholds: 100 MiB / 200 ms / 450 inserts
///   when async dedup is on, whichever trips first).
/// - `wait_for_async_insert=1` — durable ack: the call returns only after the
///   buffered block is flushed to disk, so insert errors surface to the caller.
/// - `deduplicate_blocks_in_dependent_materialized_views=1` — propagates the
///   dedup decision to the target tables of dependent materialized views, so a
///   retried batch that is dropped at the source is ALSO dropped at each MV
///   target instead of being folded in a second time (which double-counts
///   aggregates such as OHLC volume). This is only effective when each MV target
///   table ALSO has a dedup window enabled — `ReplicatedMergeTree` (on by
///   default) or a non-replicated `*MergeTree` with
///   `non_replicated_deduplication_window` set.
/// - `insert_deduplication_token='…'` — makes a retried, byte-identical batch a
///   no-op even when rows carry non-idempotent server defaults (e.g. a
///   `DEFAULT now()` ingest timestamp), which would otherwise defeat the
///   content-hash dedup. Requires a `*MergeTree` target with deduplication
///   enabled — `ReplicatedMergeTree` (on by default) or a non-replicated table
///   with `non_replicated_deduplication_window` set.
///
/// # Errors
///
/// Returns an error if `table` is empty or a row fails to serialize.
pub fn build_insert_jsoneachrow<T: DataModel>(
    table: &str,
    batch: &OBBject<T>,
    dedup_token: &str,
) -> Result<String> {
    // The table name is interpolated directly into the statement (ClickHouse
    // has no bind parameter for identifiers), so validate it is a plain
    // `[db.]table` identifier rather than escaping it. Callers pass internal
    // constants today; this guards against an identifier-injection surface if a
    // table name ever derives from external input.
    if !is_valid_identifier(table) {
        return Err(Error::Storage(format!(
            "clickhouse insert table is not a valid identifier: {table:?}"
        )));
    }
    let mut sql = format!(
        "INSERT INTO {table} SETTINGS async_insert=1, wait_for_async_insert=1, \
deduplicate_blocks_in_dependent_materialized_views=1, \
insert_deduplication_token='{token}' FORMAT JSONEachRow\n",
        token = escape_ch_string(dedup_token),
    );
    for row in &batch.rows {
        let line = serde_json::to_string(row)
            .map_err(|error| Error::Storage(format!("clickhouse serialize row: {error}")))?;
        sql.push_str(&line);
        sql.push('\n');
    }
    Ok(sql)
}

/// Stable, retry-safe deduplication token for an ingest batch.
///
/// Keyed by the protocol idempotency coordinates `(session_id, sequence)` plus
/// the destination `table`: a client retry that reuses the same session and
/// sequence yields the same token, so ClickHouse drops the duplicate block
/// rather than double-writing it (and double-counting it through any dependent
/// materialized views).
#[must_use]
pub fn ingest_dedup_token(session_id: &str, sequence: u64, table: &str) -> String {
    format!("{session_id}:{sequence}:{table}")
}

/// Content-addressed deduplication token for a streaming ingest batch.
///
/// Unlike [`ingest_dedup_token`], which keys on the protocol `(session_id,
/// sequence)` coordinates, a streaming loop has no per-batch protocol sequence:
/// a naive monotonic counter that restarts at 0 on process restart would reuse
/// tokens for *different* batches and cause ClickHouse to silently drop new
/// data. This token is instead derived from a deterministic hash of the batch's
/// row content (serialized client-side, so the server-side `DEFAULT now64()`
/// ingest timestamp does not perturb it). A retried identical batch hashes the
/// same (deduped); two distinct batches hash differently (both kept).
///
/// # Errors
///
/// Returns an error if a row fails to serialize.
pub fn batch_dedup_token<T: DataModel>(
    session_id: &str,
    table: &str,
    batch: &OBBject<T>,
) -> Result<String> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    batch.rows.len().hash(&mut hasher);
    for row in &batch.rows {
        let line = serde_json::to_string(row)
            .map_err(|error| Error::Storage(format!("clickhouse serialize row: {error}")))?;
        line.hash(&mut hasher);
    }
    Ok(format!("{session_id}:{table}:{:016x}", hasher.finish()))
}

/// True if `name` is a plain `[db.]table` identifier (ASCII letter/underscore
/// start, then letters/digits/underscore/dot). Used to guard the table name
/// that is interpolated into `INSERT` statements.
#[must_use]
fn is_valid_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Escape a string for safe interpolation inside a single-quoted ClickHouse SQL
/// literal.
fn escape_ch_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

#[async_trait]
impl<T: DataModel> WriteSink<T> for ClickHouseRecordingEngine {
    fn name(&self) -> &'static str {
        "clickhouse-recording"
    }

    async fn write_batch(&self, batch: &OBBject<T>) -> Result<WriteReceipt> {
        let mut rows = self
            .rows_written
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        *rows += batch.rows.len();
        Ok(WriteReceipt {
            sink: "clickhouse-recording",
            rows_written: batch.rows.len(),
        })
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        Ok(HealthStatus::Healthy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    struct Row {
        symbol: String,
    }

    #[test]
    fn exposes_olap_and_write_sink_contracts() {
        fn assert_olap<T: OlapEngine>() {}
        fn assert_sink<T: WriteSink<Row>>() {}

        assert_olap::<ClickHouseRecordingEngine>();
        assert_sink::<ClickHouseRecordingEngine>();
    }

    #[test]
    fn records_statements_queries_rows_and_health() {
        let engine = ClickHouseRecordingEngine::default();
        block_on_ready(engine.execute("create table bars"))
            .unwrap_or_else(|error| panic!("execute should succeed: {error}"));
        let query = block_on_ready(engine.query_json("select 1", serde_json::json!({"x": 1})))
            .unwrap_or_else(|error| panic!("query should succeed: {error}"));
        let batch = OBBject::new(
            vec![Row {
                symbol: "AAPL".to_string(),
            }],
            "fixture",
            "rows",
        );
        let receipt = block_on_ready(engine.write_batch(&batch))
            .unwrap_or_else(|error| panic!("write should succeed: {error}"));
        let health = block_on_ready(<ClickHouseRecordingEngine as WriteSink<Row>>::health_check(
            &engine,
        ))
        .unwrap_or_else(|error| panic!("health should succeed: {error}"));

        assert_eq!(
            engine
                .statements()
                .unwrap_or_else(|error| panic!("statements should read: {error}")),
            vec!["create table bars"]
        );
        assert_eq!(query["engine"], "clickhouse-recording");
        assert_eq!(receipt.rows_written, 1);
        assert_eq!(
            engine
                .rows_written()
                .unwrap_or_else(|error| panic!("row count should read: {error}")),
            1
        );
        assert_eq!(health, HealthStatus::Healthy);
        assert!(block_on_ready(engine.execute("")).is_err());
    }

    #[test]
    fn build_insert_emits_idempotent_jsoneachrow_header() {
        let batch = OBBject::new(
            vec![
                Row {
                    symbol: "AAPL".to_string(),
                },
                Row {
                    symbol: "MSFT".to_string(),
                },
            ],
            "yahoo",
            "equity_historical",
        );
        let sql = build_insert_jsoneachrow(
            "raw.equity_historical",
            &batch,
            "sess-1:7:raw.equity_historical",
        )
        .unwrap_or_else(|error| panic!("insert should build: {error}"));

        assert!(sql.starts_with("INSERT INTO raw.equity_historical SETTINGS "));
        assert!(sql.contains("async_insert=1"));
        assert!(sql.contains("wait_for_async_insert=1"));
        assert!(sql.contains("deduplicate_blocks_in_dependent_materialized_views=1"));
        assert!(sql.contains("insert_deduplication_token='sess-1:7:raw.equity_historical'"));
        assert!(sql.contains("FORMAT JSONEachRow"));
        // One JSON object per row, newline-delimited after the header line.
        assert!(sql.contains("{\"symbol\":\"AAPL\"}"));
        assert!(sql.contains("{\"symbol\":\"MSFT\"}"));
        assert_eq!(sql.matches("\"symbol\"").count(), 2);
    }

    #[test]
    fn build_insert_rejects_empty_table_and_escapes_token() {
        let batch = OBBject::new(
            vec![Row {
                symbol: "AAPL".to_string(),
            }],
            "yahoo",
            "equity_historical",
        );
        assert!(build_insert_jsoneachrow("", &batch, "tok").is_err());
        // Identifier injection surface is rejected, not escaped.
        assert!(build_insert_jsoneachrow("raw.t; drop table x", &batch, "tok").is_err());
        assert!(build_insert_jsoneachrow("raw.t--x", &batch, "tok").is_err());
        assert!(build_insert_jsoneachrow("1bad", &batch, "tok").is_err());

        let sql = build_insert_jsoneachrow("raw.t", &batch, "a'b\\c")
            .unwrap_or_else(|error| panic!("insert should build: {error}"));
        assert!(sql.contains("insert_deduplication_token='a\\'b\\\\c'"));
    }

    #[test]
    fn batch_dedup_token_is_content_stable_and_distinct() {
        let a1 = OBBject::new(
            vec![Row {
                symbol: "AAPL".to_string(),
            }],
            "stream",
            "stream_ingest",
        );
        // Same content (a retried batch) -> same token, regardless of batch order.
        let t1 = batch_dedup_token("sess", "raw.tick", &a1).expect("token");
        let t1_retry = batch_dedup_token("sess", "raw.tick", &a1).expect("token");
        assert_eq!(t1, t1_retry, "identical content must yield a stable token");

        // Different content -> different token (no silent dedup of new data).
        let a2 = OBBject::new(
            vec![Row {
                symbol: "MSFT".to_string(),
            }],
            "stream",
            "stream_ingest",
        );
        let t2 = batch_dedup_token("sess", "raw.tick", &a2).expect("token");
        assert_ne!(t1, t2, "distinct content must yield distinct tokens");
        assert!(t1.starts_with("sess:raw.tick:"));
    }

    #[test]
    fn ingest_dedup_token_is_stable_across_retries() {
        let first = ingest_dedup_token("sess-1", 7, "raw.equity_historical");
        let retry = ingest_dedup_token("sess-1", 7, "raw.equity_historical");
        let other = ingest_dedup_token("sess-1", 8, "raw.equity_historical");

        assert_eq!(first, retry, "same coordinates must yield the same token");
        assert_ne!(first, other, "a new sequence must yield a new token");
        assert_eq!(first, "sess-1:7:raw.equity_historical");
    }

    struct NoopWaker;

    impl Wake for NoopWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWaker));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("test future should be ready without an executor"),
        }
    }
}
