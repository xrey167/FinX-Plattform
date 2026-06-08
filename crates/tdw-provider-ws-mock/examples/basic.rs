//! Offline mock-streamer example: drain the deterministic `subscribe` stream
//! and read the snapshot. No network, no feature flags, no async runtime — the
//! mock stream is always-ready, so we poll it with a no-op waker, mirroring the
//! crate's own tests.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-ws-mock --example basic
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use futures_core::Stream;
use tdw_core::{Credentials, Result, Streamer};
use tdw_provider_ws_mock::{EquityTickQuery, MockEquityStreamer};

fn main() {
    let streamer = MockEquityStreamer;

    // Snapshot: symbol is validated and upper-cased.
    let snapshot_query = EquityTickQuery {
        symbol: " aapl ".to_string(),
    };
    let rows = block_on_ready(streamer.snapshot(&snapshot_query, &Credentials::default()))
        .expect("snapshot");
    for bar in &rows {
        println!(
            "snapshot {} close={} source={}",
            bar.symbol, bar.close, bar.source
        );
    }

    // Subscribe: drain the deterministic single-bar stream.
    let stream_query = EquityTickQuery {
        symbol: "msft".to_string(),
    };
    let mut stream = block_on_ready(streamer.subscribe(stream_query, &Credentials::default()))
        .expect("subscribe");
    while let Some(item) = poll_next_ready(stream.as_mut()) {
        let bar = item.expect("stream row");
        println!("stream {} @ {}", bar.symbol, bar.ts);
    }
}

// --- always-ready executor shims (no tokio dependency) -----------------------

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
        Poll::Pending => panic!("offline future should be ready without an executor"),
    }
}

fn poll_next_ready<T>(
    stream: Pin<&mut (dyn Stream<Item = Result<T>> + Send)>,
) -> Option<Result<T>> {
    let waker = Waker::from(Arc::new(NoopWaker));
    let mut context = Context::from_waker(&waker);
    match stream.poll_next(&mut context) {
        Poll::Ready(item) => item,
        Poll::Pending => panic!("offline stream should be ready without an executor"),
    }
}
