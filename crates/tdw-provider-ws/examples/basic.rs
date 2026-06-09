//! Offline websocket example: decode JSON-array and NDJSON tick frames with the
//! pure `decode_frame`, then drain the deterministic offline `subscribe` stream
//! (the no-`ws`-feature path). No socket is opened and no async runtime is
//! required — the offline stream is always-ready, so we poll it with a no-op
//! waker, mirroring the crate's own tests.
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-ws --example basic
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use futures_core::Stream;
use tdw_core::{Credentials, Result, Streamer};
use tdw_provider_ws::{WsTickQuery, WsTickStreamer, decode_frame};

fn main() {
    // --- Pure frame decoding -------------------------------------------------
    let array_frame = r#"[
        {"symbol":"AAPL","venue":"WS","ts":"2026-05-21T20:00:00Z","price":100.5,"size":10.0},
        {"symbol":"MSFT","venue":"WS","ts":"2026-05-21T20:00:01Z","price":420.0,"size":2.0}
    ]"#;
    let ticks = decode_frame(array_frame).expect("array frame decodes");
    println!("array frame -> {} ticks", ticks.len());

    let ndjson_frame = "\n{\"symbol\":\"NVDA\",\"venue\":\"WS\",\"ts\":\"2026-05-21T20:00:02Z\",\"price\":900.0,\"size\":1.0}\n\n";
    let ndjson_ticks = decode_frame(ndjson_frame).expect("ndjson frame decodes");
    println!("ndjson frame -> {} ticks", ndjson_ticks.len());

    // --- Deterministic offline stream consumption ----------------------------
    let streamer = WsTickStreamer;
    let query = WsTickQuery {
        url: "wss://example.invalid/ws".to_string(),
        symbol: "AAPL".to_string(),
    };
    let mut stream =
        block_on_ready(streamer.subscribe(query, &Credentials::default())).expect("subscribe");
    while let Some(item) = poll_next_ready(stream.as_mut()) {
        let tick = item.expect("offline stream row");
        println!("stream tick {} @ {}", tick.symbol, tick.price);
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
