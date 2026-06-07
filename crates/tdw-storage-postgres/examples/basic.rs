//! Offline `PostgresRecordingEngine` round-trip: record a statement and read
//! back the synthetic JSON the recording engine returns. No network, no docker.
//!
//! The recording engine's futures resolve without an executor, so this example
//! drives them with a tiny no-op waker instead of pulling in a runtime (tokio is
//! only a dependency under the `postgres` feature).
//!
//! Run with: `cargo run -p tdw-storage-postgres --example basic`

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use serde_json::json;
use tdw_core::RelationalEngine;
use tdw_storage_postgres::PostgresRecordingEngine;

fn main() -> tdw_core::Result<()> {
    let engine = PostgresRecordingEngine::default();

    // Record a write, then "fetch" — the recording engine echoes the call back.
    let affected = block_on_ready(engine.execute(
        "insert into raw.market_data_bar (symbol) values ($1)",
        json!(["AAPL"]),
    ))?;
    let rows = block_on_ready(engine.fetch_json("select * from raw.market_data_bar", json!([])))?;

    assert_eq!(affected, 1);
    assert_eq!(rows[0]["engine"], "postgres-recording");
    println!(
        "round-trip ok: recorded statements = {:?}, fetched engine = {}",
        engine.statements()?,
        rows[0]["engine"]
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
