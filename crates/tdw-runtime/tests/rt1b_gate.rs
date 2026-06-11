//! Integration tests for G008 theme 10 — RT1b: fake-streaming gate.
#![allow(clippy::doc_markdown)] // gate/env-var names in backtick spans are intentional prose
//!
//! **Gate-closed path** (no env var): covered by the
//! `run_streaming_refuses_without_builder_opt_in` unit test in `lib.rs`.
//!
//! **Gate-open path** (env var set): run with:
//!
//! ```text
//! TDW_ALLOW_FAKE_STREAMING=1 cargo test -p tdw-runtime --test rt1b_gate
//! ```
//!
//! When `TDW_ALLOW_FAKE_STREAMING` is not set the opt-in test skips gracefully.
//!
//! The `MockFetcher` is reproduced here (it cannot be re-exported from the
//! lib) so these tests remain self-contained.

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;
use tdw_core::{Credentials, Fetcher, Result};
use tdw_runtime::CommandRunner;

#[derive(
    Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
struct Query {
    symbol: String,
}

#[derive(
    Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
struct Row {
    symbol: String,
}

struct MockFetcher;

#[async_trait]
impl Fetcher<Query, Row> for MockFetcher {
    const PROVIDER: &'static str = "mock";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<Query> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| tdw_core::Error::InvalidQuery("missing symbol".to_string()))?;
        Ok(Query { symbol: symbol.to_string() })
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

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut cx = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
    }
}

/// Without opt-in, `run_streaming` must refuse with a `Provider` error naming
/// `TDW_ALLOW_FAKE_STREAMING`.
///
/// Skip gracefully if `TDW_ALLOW_FAKE_STREAMING=1` is already set in the
/// environment (developer override active — gate-closed path not testable).
#[test]
fn run_streaming_refuses_without_opt_in() {
    if std::env::var("TDW_ALLOW_FAKE_STREAMING").as_deref() == Ok("1") {
        eprintln!(
            "SKIP: TDW_ALLOW_FAKE_STREAMING=1 is set; gate-closed path not testable"
        );
        return;
    }
    let runner = CommandRunner::default();
    let fetcher = MockFetcher;
    let result = block_on(runner.run_streaming(&fetcher, serde_json::json!({"symbol": "AAPL"})));
    match result {
        Err(tdw_core::Error::Provider(msg)) => {
            assert!(
                msg.contains("TDW_ALLOW_FAKE_STREAMING"),
                "error must name the bypass flag, got: {msg}"
            );
        }
        Err(other) => panic!("expected Provider error, got a different error: {other}"),
        Ok(_) => panic!("expected Provider error, got Ok"),
    }
}

/// With `TDW_ALLOW_FAKE_STREAMING=1` (env-var bypass), `run_streaming` succeeds.
///
/// Skip when the env var is absent — the opt-in path cannot be exercised without
/// it.  Run with:
///
/// ```text
/// TDW_ALLOW_FAKE_STREAMING=1 cargo test -p tdw-runtime --test rt1b_gate
/// ```
#[test]
fn run_streaming_allowed_via_env_var() {
    if std::env::var("TDW_ALLOW_FAKE_STREAMING").as_deref() != Ok("1") {
        eprintln!(
            "SKIP: set TDW_ALLOW_FAKE_STREAMING=1 to run the env-var bypass test"
        );
        return;
    }
    let runner = CommandRunner::default(); // builder flag NOT set — env var triggers
    let fetcher = MockFetcher;
    let mut stream = block_on(runner.run_streaming(
        &fetcher,
        serde_json::json!({"symbol": "AAPL"}),
    ))
    .unwrap_or_else(|e| panic!("env-var bypass must allow streaming: {e}"));

    // Consume the stream — must produce two Progress items then Done.
    let waker = Waker::from(Arc::new(NoopWake));
    let mut cx = Context::from_waker(&waker);
    let mut items = Vec::new();
    loop {
        match stream.as_mut().poll_next(&mut cx) {
            Poll::Ready(Some(item)) => items.push(item),
            Poll::Ready(None) | Poll::Pending => break,
        }
    }
    assert_eq!(items.len(), 3, "expected 2 Progress + 1 Done: got {}", items.len());
}
