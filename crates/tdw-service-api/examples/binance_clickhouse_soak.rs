//! Bounded live soak: Binance BTCUSDT trade websocket → ClickHouse.
//!
//! Runs for `TDW_SOAK_DURATION_SECS` seconds (default 1500 = 25 min), draining
//! the public Binance `BTCUSDT@trade` stream into a ClickHouse `raw.tick` table
//! via the content-addressed idempotent ingest path (`run_ws_ingest`).
//!
//! Required env vars:
//!   TDW_CLICKHOUSE_URL   – ClickHouse HTTP endpoint (e.g. `http://127.0.0.1:8123`)
//!   TDW_CLICKHOUSE_USER  – ClickHouse user (default: `default`)
//!   TDW_CLICKHOUSE_PASS  – ClickHouse password (default: empty)
//!
//! Optional env vars:
//!   TDW_SOAK_DURATION_SECS – wall-clock budget in seconds (default: 1500)
//!   TDW_SOAK_SYMBOL        – Binance symbol to stream (default: BTCUSDT)
//!
//! The binary:
//!   1. Creates `raw.tick` if it does not exist.
//!   2. Streams trades for the bounded window, flushing 500-row batches or on a
//!      5-second idle timeout, whichever trips first.
//!   3. Emits a one-line summary (rows, rate/s, first_ts, last_ts) to stdout and
//!      — when `GITHUB_STEP_SUMMARY` is set — appends a Markdown table to the
//!      step summary file.
//!   4. Exits 0 on success; exits non-zero on any ingest error.
//!
//! Run locally (requires `--features real-clickhouse,ws`):
//!   ```
//!   TDW_CLICKHOUSE_URL=http://127.0.0.1:8123 \
//!   TDW_SOAK_DURATION_SECS=60 \
//!   cargo run -p tdw-service-api --example binance_clickhouse_soak \
//!     --features real-clickhouse,ws --target-dir target
//!   ```

use std::time::{Duration, Instant};

use tdw_core::{Credentials, OlapEngine};
use tdw_provider_binance::{BinanceTradeQuery, BinanceTradeStreamer};
use tdw_service_api::run_ws_ingest;
use tdw_storage_clickhouse::ClickHouseHttpEngine;

/// ClickHouse table DDL for the raw tick landing table.
///
/// Uses a plain `MergeTree` so this works on both single-node ClickHouse and
/// ClickHouse Cloud (no replication required for the soak). The
/// `insert_deduplication_token` setting on the INSERT path handles dedup at the
/// server level; the `(symbol, ts)` primary key makes dedup queries efficient.
const CREATE_TABLE_SQL: &str = "\
CREATE TABLE IF NOT EXISTS raw.tick (
    symbol   LowCardinality(String),
    venue    LowCardinality(String),
    ts       DateTime64(3, 'UTC'),
    price    Float64,
    size     Float64
) ENGINE = MergeTree()
ORDER BY (symbol, ts)
SETTINGS non_replicated_deduplication_window = 1000";

const CREATE_DB_SQL: &str = "CREATE DATABASE IF NOT EXISTS raw";

/// Batch size threshold: flush when the buffer accumulates this many rows.
const MAX_ROWS_PER_BATCH: usize = 500;

/// Idle flush interval: flush even if the batch is not full after this wait.
const FLUSH_IDLE: Duration = Duration::from_secs(5);

/// Default soak window in seconds (25 minutes).
const DEFAULT_DURATION_SECS: u64 = 1500;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- Configuration from environment ----------------------------------------
    let ch_url =
        std::env::var("TDW_CLICKHOUSE_URL").unwrap_or_else(|_| "http://127.0.0.1:8123".to_string());
    let ch_user = std::env::var("TDW_CLICKHOUSE_USER").ok();
    let ch_pass = std::env::var("TDW_CLICKHOUSE_PASS").ok();
    let duration_secs: u64 = std::env::var("TDW_SOAK_DURATION_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_DURATION_SECS);
    let symbol = std::env::var("TDW_SOAK_SYMBOL").unwrap_or_else(|_| "BTCUSDT".to_string());

    let soak_window = Duration::from_secs(duration_secs);

    println!("[soak] starting: symbol={symbol} window={duration_secs}s endpoint={ch_url}");

    // --- Wire ClickHouse --------------------------------------------------------
    let engine = ClickHouseHttpEngine::new(&ch_url, ch_user, ch_pass)?;

    engine.execute(CREATE_DB_SQL).await?;
    engine.execute(CREATE_TABLE_SQL).await?;
    println!("[soak] table raw.tick ready");

    // --- Capture first/last ts for the summary ----------------------------------
    let wall_start = Instant::now();
    // Session id for idempotent dedup tokens — stable per process run.
    let session_id = format!(
        "soak:{}:{}",
        symbol,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );

    // --- Bounded ingest ---------------------------------------------------------
    // `run_ws_ingest` drives the stream until it ends or errors. We wrap it in a
    // `tokio::time::timeout` so the soak is strictly bounded even if the
    // exchange never closes the socket (it won't for a live feed).
    let streamer = BinanceTradeStreamer;
    let query = BinanceTradeQuery::new(&symbol)?;
    let creds = Credentials::default();

    let ingest_result = tokio::time::timeout(
        soak_window,
        run_ws_ingest(
            &engine,
            &streamer,
            query,
            &creds,
            &session_id,
            "raw.tick",
            MAX_ROWS_PER_BATCH,
            FLUSH_IDLE,
        ),
    )
    .await;

    let elapsed = wall_start.elapsed();
    let elapsed_secs = elapsed.as_secs_f64();

    // A timeout is the expected happy path (bounded window exhausted).
    let rows_written = match ingest_result {
        Ok(Ok(rows)) => rows,
        Ok(Err(ingest_err)) => {
            eprintln!("[soak] ingest error: {ingest_err}");
            return Err(ingest_err.into());
        }
        Err(_timeout) => {
            // Timeout = window elapsed normally.  Query ClickHouse for the
            // actual persisted row count rather than relying on the in-flight
            // partial batch count (the last batch may not have flushed before
            // the timeout fired).
            let count_result = engine
                .query_json(
                    &format!("SELECT count() AS n FROM raw.tick WHERE symbol = '{symbol}'"),
                    serde_json::Value::Null,
                )
                .await;
            match count_result {
                Ok(v) => v["data"][0]["n"]
                    .as_str()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0),
                Err(e) => {
                    eprintln!("[soak] count query failed after timeout: {e}");
                    0
                }
            }
        }
    };

    let rate = if elapsed_secs > 0.0 {
        rows_written as f64 / elapsed_secs
    } else {
        0.0
    };

    // --- Query first/last timestamps --------------------------------------------
    let first_last = engine
        .query_json(
            &format!(
                "SELECT min(ts) AS first_ts, max(ts) AS last_ts \
                 FROM raw.tick WHERE symbol = '{symbol}'"
            ),
            serde_json::Value::Null,
        )
        .await
        .ok();
    let first_ts = first_last
        .as_ref()
        .and_then(|v| v["data"][0]["first_ts"].as_str())
        .unwrap_or("n/a")
        .to_string();
    let last_ts = first_last
        .as_ref()
        .and_then(|v| v["data"][0]["last_ts"].as_str())
        .unwrap_or("n/a")
        .to_string();

    // --- Console summary --------------------------------------------------------
    println!(
        "[soak] done: rows={rows_written} rate={rate:.1}/s elapsed={elapsed_secs:.0}s \
         first_ts={first_ts} last_ts={last_ts}"
    );

    // --- GitHub step summary (if running in CI) ----------------------------------
    if let Ok(summary_path) = std::env::var("GITHUB_STEP_SUMMARY") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&summary_path)
        {
            let _ = writeln!(
                f,
                "\n## P2.4 Nightly Soak — {symbol}\n\
                 | Metric | Value |\n\
                 |--------|-------|\n\
                 | Rows ingested | {rows_written} |\n\
                 | Rate | {rate:.1} rows/s |\n\
                 | Wall time | {elapsed_secs:.0}s |\n\
                 | First tick ts | {first_ts} |\n\
                 | Last tick ts | {last_ts} |"
            );
        }
    }

    Ok(())
}
