//! Offline, no-network example for `tdw-runtime`.
//!
//! Defines a deterministic in-example `Fetcher` (no provider crate, no network),
//! registers it with a `CommandRunner`, then exercises both the terminal `run`
//! path and the progress-wrapped `run_streaming` path. The crate does not depend
//! on tokio, so the example drives the futures with a tiny no-op-waker executor
//! (the same approach the crate's own tests use).
//!
//! Run with: `cargo run -p tdw-runtime --example tdw_runtime_basic`

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::{Value, json};
use tdw_core::{
    Credentials, DataModel, Fetcher, ProgressOrResult, ProgressStream, ProviderRegistry,
    RegistryEntry, Result,
};
use tdw_runtime::CommandRunner;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
struct Query {
    symbol: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
struct Row {
    symbol: String,
}

/// A deterministic, offline fetcher: echoes the requested symbol back as one row.
struct MockFetcher;

#[async_trait]
impl Fetcher<Query, Row> for MockFetcher {
    const PROVIDER: &'static str = "example";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<Query> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| tdw_core::Error::InvalidQuery("missing symbol".to_string()))?;
        Ok(Query {
            symbol: symbol.to_string(),
        })
    }

    async fn extract_data(&self, query: &Query, _creds: &Credentials) -> Result<Bytes> {
        Ok(Bytes::from(query.symbol.clone()))
    }

    fn transform_data(&self, _query: &Query, raw: Bytes) -> Result<Vec<Row>> {
        let symbol = String::from_utf8(raw.to_vec())
            .map_err(|e| tdw_core::Error::Provider(e.to_string()))?;
        Ok(vec![Row { symbol }])
    }
}

fn main() -> Result<()> {
    // Register the provider so the runner advertises it, then build the runner.
    let mut registry = ProviderRegistry::default();
    registry.register(RegistryEntry::fetcher(
        MockFetcher::PROVIDER,
        MockFetcher::ENDPOINT,
    ))?;
    let runner = CommandRunner::new(registry).with_credentials(Credentials::default());
    println!(
        "registered providers: {}",
        runner.registered_providers().len()
    );

    // 1. Terminal fetch.
    let object = block_on(runner.run(&MockFetcher, json!({ "symbol": "AAPL" })))?;
    println!(
        "fetch -> provider={} rows={} first={}",
        object.provider,
        object.rows.len(),
        object.rows[0].symbol,
    );

    // 2. Streaming fetch: the runtime wraps the terminal result in a
    //    deterministic start/done progress stream.
    let mut stream = block_on(runner.run_streaming(&MockFetcher, json!({ "symbol": "MSFT" })))?;
    let mut stages = Vec::new();
    while let Some(item) = poll_next(&mut stream) {
        match item? {
            ProgressOrResult::Progress {
                stage, fraction, ..
            } => stages.push(format!("{stage}:{fraction:.1}")),
            ProgressOrResult::Done(object) => {
                stages.push(format!("done:{}", object.rows[0].symbol));
            }
            _ => {}
        }
    }
    println!("stream stages: {stages:?}");

    Ok(())
}

// --- minimal no-op-waker executor (the crate has no tokio dependency) ---

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut cx = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
    }
}

fn poll_next<T: DataModel>(stream: &mut ProgressStream<T>) -> Option<Result<ProgressOrResult<T>>> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut cx = Context::from_waker(&waker);
    match stream.as_mut().poll_next(&mut cx) {
        Poll::Ready(item) => item,
        Poll::Pending => None,
    }
}
