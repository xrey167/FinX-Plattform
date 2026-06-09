//! Offline `InMemoryBrokerSink` round-trip: write a batch and read back the
//! recorded `JSONEachRow` messages. No network, no docker — the default in-memory
//! sink is always available.
//!
//! `WriteSink::write_batch` is async but the in-memory sink resolves
//! immediately, so this example drives it with a tiny no-op waker rather than
//! pulling in a runtime.
//!
//! Run with: `cargo run -p tdw-storage-broker --example tdw-storage-broker-basic`

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tdw_core::{OBBject, WriteSink};
use tdw_storage_broker::InMemoryBrokerSink;

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct Tick {
    symbol: String,
    price: f64,
}

fn main() -> tdw_core::Result<()> {
    let sink = InMemoryBrokerSink::new("tdw.tick");
    let batch = OBBject::new(
        vec![
            Tick {
                symbol: "AAPL".to_string(),
                price: 1.0,
            },
            Tick {
                symbol: "MSFT".to_string(),
                price: 2.0,
            },
        ],
        "ws",
        "equity_ticks",
    );

    let receipt = block_on_ready(sink.write_batch(&batch))?;
    let messages = sink.messages()?;

    assert_eq!(receipt.rows_written, 2);
    assert_eq!(messages.len(), 2);
    println!(
        "produced {} messages to {}: first payload = {}",
        messages.len(),
        sink.topic(),
        messages[0].payload
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
        Poll::Pending => panic!("in-memory sink future should be ready without an executor"),
    }
}
