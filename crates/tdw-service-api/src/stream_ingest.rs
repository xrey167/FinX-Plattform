use std::time::Duration;

use futures_util::StreamExt;
use tdw_core::{Credentials, DataModel, DataStream, OlapEngine, QueryParams, Result, Streamer};
use tdw_storage_clickhouse::{batch_dedup_token, build_insert_jsoneachrow};

/// Provider label stamped on every streamed ingest batch envelope.
const STREAM_PROVIDER: &str = "stream";
/// Endpoint label stamped on every streamed ingest batch envelope.
const STREAM_ENDPOINT: &str = "stream_ingest";

/// Drain a streaming source into a ClickHouse `OlapEngine`, flushing batches
/// whenever the buffer reaches `max_rows` rows or `max_wait` elapses since the
/// last flush, whichever trips first.
///
/// Each flush is written as an idempotent `INSERT … FORMAT JSONEachRow`
/// statement carrying a CONTENT-ADDRESSED `insert_deduplication_token` from
/// [`batch_dedup_token`] (a hash of the batch's rows). A streaming loop has no
/// per-batch protocol sequence, so a monotonic counter would restart at 0 on
/// process restart and reuse tokens for *different* batches — silently dropping
/// new data. Hashing the content instead makes a retried identical batch dedup
/// and two distinct batches both persist.
///
/// Returns the total number of rows written across all flushes.
///
/// # Errors
///
/// Returns an error if the stream yields an error item, if a batch fails to
/// serialize into a JSONEachRow statement, or if the engine rejects an INSERT.
pub async fn run_stream_ingest<T: DataModel>(
    olap: &dyn OlapEngine,
    session_id: &str,
    table: &str,
    mut stream: DataStream<T>,
    max_rows: usize,
    max_wait: Duration,
) -> Result<usize> {
    let mut buffer: Vec<T> = Vec::new();
    let mut total: usize = 0;

    loop {
        let next = if buffer.is_empty() {
            // Nothing buffered: block until the stream produces (or ends). A
            // wait timeout with an empty buffer is a no-op, so skip the timer.
            stream.next().await
        } else {
            match tokio::time::timeout(max_wait, stream.next()).await {
                Ok(item) => item,
                Err(_elapsed) => {
                    flush(olap, session_id, table, &mut buffer, &mut total).await?;
                    continue;
                }
            }
        };

        match next {
            Some(item) => {
                buffer.push(item?);
                if buffer.len() >= max_rows {
                    flush(olap, session_id, table, &mut buffer, &mut total).await?;
                }
            }
            None => {
                if !buffer.is_empty() {
                    flush(olap, session_id, table, &mut buffer, &mut total).await?;
                }
                break;
            }
        }
    }

    Ok(total)
}

/// Subscribe to a [`Streamer`] and drain its [`DataStream`] into ClickHouse via
/// [`run_stream_ingest`].
///
/// This is the end-to-end driver that wires a provider subscription to the
/// content-addressed dedup ingest loop: it calls `streamer.subscribe(query,
/// creds)` to obtain the live stream, then hands it to [`run_stream_ingest`]
/// with the same flush thresholds.
///
/// Returns the total number of rows written across all flushes.
///
/// # Errors
///
/// Returns an error if the subscription fails, the stream yields an error item,
/// a batch fails to serialize, or the engine rejects an INSERT.
#[allow(clippy::too_many_arguments)]
pub async fn run_ws_ingest<S, Q, D>(
    olap: &dyn OlapEngine,
    streamer: &S,
    query: Q,
    creds: &Credentials,
    session_id: &str,
    table: &str,
    max_rows: usize,
    max_wait: Duration,
) -> Result<usize>
where
    S: Streamer<Q, D>,
    Q: QueryParams,
    D: DataModel,
{
    let stream = streamer.subscribe(query, creds).await?;
    run_stream_ingest(olap, session_id, table, stream, max_rows, max_wait).await
}

async fn flush<T: DataModel>(
    olap: &dyn OlapEngine,
    session_id: &str,
    table: &str,
    buffer: &mut Vec<T>,
    total: &mut usize,
) -> Result<()> {
    if buffer.is_empty() {
        return Ok(());
    }
    let rows = std::mem::take(buffer);
    let count = rows.len();
    let batch = tdw_core::OBBject::new(rows, STREAM_PROVIDER, STREAM_ENDPOINT);
    let token = batch_dedup_token(session_id, table, &batch)?;
    let sql = build_insert_jsoneachrow(table, &batch, &token)?;
    olap.execute(&sql).await?;
    *total += count;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use futures_util::Stream;
    use tdw_core::Result as CoreResult;
    use tdw_domain::Tick;
    use tdw_provider_ws::{WsTickQuery, WsTickStreamer};
    use tdw_storage_clickhouse::ClickHouseRecordingEngine;

    struct VecStream<T> {
        items: VecDeque<CoreResult<T>>,
    }

    impl<T> VecStream<T> {
        fn new(items: Vec<T>) -> Self {
            Self {
                items: items.into_iter().map(Ok).collect(),
            }
        }
    }

    impl<T> Unpin for VecStream<T> {}

    impl<T> Stream for VecStream<T> {
        type Item = CoreResult<T>;

        fn poll_next(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<<Self as Stream>::Item>> {
            Poll::Ready(self.get_mut().items.pop_front())
        }
    }

    fn tick(symbol: &str) -> Tick {
        Tick {
            symbol: symbol.to_string(),
            venue: "WS".to_string(),
            ts: "2026-05-21T20:00:00Z".to_string(),
            price: 100.0,
            size: 1.0,
        }
    }

    #[tokio::test]
    async fn drains_stream_into_distinct_dedup_batches() {
        let engine = ClickHouseRecordingEngine::default();
        let ticks = vec![
            tick("AAPL"),
            tick("MSFT"),
            tick("NVDA"),
            tick("AMZN"),
            tick("TSLA"),
        ];
        let stream: DataStream<Tick> = Box::pin(VecStream::new(ticks));

        let total = run_stream_ingest(
            &engine,
            "sess-stream-1",
            "raw.tick",
            stream,
            2,
            Duration::from_secs(60),
        )
        .await
        .unwrap_or_else(|error| panic!("stream ingest should succeed: {error}"));

        // 5 rows, max_rows=2 -> flushes of 2, 2, 1 = three INSERT statements.
        assert_eq!(total, 5);
        let statements = engine
            .statements()
            .unwrap_or_else(|error| panic!("statements should read: {error}"));
        assert_eq!(statements.len(), 3);
        for statement in &statements {
            assert!(statement.contains("FORMAT JSONEachRow"));
            assert!(statement.starts_with("INSERT INTO raw.tick SETTINGS "));
            // Content-addressed token, scoped to (session, table).
            assert!(statement.contains("insert_deduplication_token='sess-stream-1:raw.tick:"));
        }
        // Distinct batch content -> distinct tokens (and statements), so no batch
        // is silently deduped against another.
        assert_ne!(statements[0], statements[1]);
        assert_ne!(statements[1], statements[2]);
        assert_ne!(statements[0], statements[2]);
    }

    #[tokio::test]
    async fn ws_ingest_subscribes_and_persists_offline_tick() {
        let engine = ClickHouseRecordingEngine::default();

        let total = run_ws_ingest(
            &engine,
            &WsTickStreamer,
            WsTickQuery {
                url: String::new(),
                symbol: "AAPL".into(),
            },
            &Credentials::default(),
            "stream:ws:equity_ticks",
            "raw.tick",
            10,
            Duration::from_millis(50),
        )
        .await
        .unwrap_or_else(|error| panic!("ws ingest should succeed: {error}"));

        assert_eq!(total, 1);
        let statements = engine
            .statements()
            .unwrap_or_else(|error| panic!("statements should read: {error}"));
        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("INSERT INTO raw.tick"));
        assert!(statements[0].contains("FORMAT JSONEachRow"));
        assert!(statements[0].contains("\"symbol\":\"AAPL\""));
    }

    #[tokio::test]
    async fn empty_stream_writes_nothing() {
        let engine = ClickHouseRecordingEngine::default();
        let stream: DataStream<Tick> = Box::pin(VecStream::new(Vec::new()));

        let total = run_stream_ingest(
            &engine,
            "sess-stream-2",
            "raw.tick",
            stream,
            4,
            Duration::from_secs(60),
        )
        .await
        .unwrap_or_else(|error| panic!("empty ingest should succeed: {error}"));

        assert_eq!(total, 0);
        assert!(
            engine
                .statements()
                .unwrap_or_else(|error| panic!("statements should read: {error}"))
                .is_empty()
        );
    }
}
