//! Offline `ClickHouseRecordingEngine` round-trip: record a DDL statement and
//! read back the synthetic JSON the recording engine returns. No network, no
//! docker.
//!
//! The recording engine's futures resolve without an executor, so this example
//! drives them with a tiny no-op waker instead of pulling in a runtime (tokio is
//! only a dependency under the `clickhouse` feature).
//!
//! Run with: `cargo run -p tdw-storage-clickhouse --example tdw-storage-clickhouse-basic`

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use serde_json::json;
use tdw_core::OlapEngine;
use tdw_storage_clickhouse::ClickHouseRecordingEngine;

fn main() -> tdw_core::Result<()> {
    let engine = ClickHouseRecordingEngine::default();

    block_on_ready(engine.execute(
        "create table analytics.ohlc (ts DateTime, close Float64) engine = MergeTree order by ts",
    ))?;
    let result =
        block_on_ready(engine.query_json("select count() from analytics.ohlc", json!({})))?;

    assert_eq!(result["engine"], "clickhouse-recording");
    println!(
        "round-trip ok: recorded statements = {:?}, query engine = {}",
        engine.statements()?,
        result["engine"]
    );
    Ok(())
}

struct NoopWaker;

impl Wake for NoopWaker {
    fn wake(self: Arc<Self>) {}
}

/// Drive a future that is known to resolve immediately (no real I/O).
fn block_on_ready<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWaker));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("recording-engine future should be ready without an executor"),
    }
}
