//! Offline Yahoo example: drive the always-compiled `YahooEquityHistoricalFetcher`
//! through the full `transform_query` -> `extract_data` -> `transform_data`
//! pipeline. Its `extract_data` synthesises a deterministic bar, so no network
//! and no feature flags are required. The async `extract_data` resolves
//! immediately, so we poll it with a no-op waker (no tokio dependency).
//!
//! Run with:
//!
//! ```bash
//! cargo run -p tdw-provider-yahoo --example basic
//! ```

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use tdw_core::{Credentials, Fetcher};
use tdw_provider_yahoo::YahooEquityHistoricalFetcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher = YahooEquityHistoricalFetcher;

    // transform_query: symbol validation is shared with tdw-provider-fileset.
    let query = YahooEquityHistoricalFetcher::transform_query(serde_json::json!({
        "symbol": "AAPL"
    }))?;
    println!("validated symbol = {}", query.symbol);

    // extract_data + transform_data, offline.
    let raw = block_on_ready(fetcher.extract_data(&query, &Credentials::default()))?;
    let rows = fetcher.transform_data(&query, raw)?;
    for bar in &rows {
        println!(
            "{} {} O={} H={} L={} C={} V={}",
            bar.symbol, bar.date, bar.open, bar.high, bar.low, bar.close, bar.volume
        );
    }

    Ok(())
}

// --- always-ready executor shim (no tokio dependency) ------------------------

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
