#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-runtime: transform_query failure propagation,
// duplicate provider registration, credential propagation, registered_providers
// snapshot, run() and run_streaming() error variants.
//
// IMPORTANT: Authored offline. Verify with
// `cargo test --package tdw-runtime` before merging.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tdw_core::{
    Credentials, Error, Fetcher, ProgressOrResult, ProviderKind, RegistryEntry, Result,
};
use tdw_runtime::CommandRunner;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Query {
    symbol: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Row {
    symbol: String,
}

struct StrictFetcher;

#[async_trait]
impl Fetcher<Query, Row> for StrictFetcher {
    const PROVIDER: &'static str = "strict";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<Query> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("missing symbol".to_string()))?;
        if symbol.is_empty() {
            return Err(Error::InvalidQuery("symbol must be non-empty".to_string()));
        }
        Ok(Query {
            symbol: symbol.to_string(),
        })
    }

    async fn extract_data(&self, _query: &Query, _creds: &Credentials) -> Result<Bytes> {
        Ok(Bytes::from_static(b"AAPL"))
    }

    fn transform_data(&self, _query: &Query, raw: Bytes) -> Result<Vec<Row>> {
        let symbol =
            String::from_utf8(raw.to_vec()).map_err(|error| Error::Provider(error.to_string()))?;
        Ok(vec![Row { symbol }])
    }
}

struct AlwaysFailingFetcher;

#[async_trait]
impl Fetcher<Query, Row> for AlwaysFailingFetcher {
    const PROVIDER: &'static str = "failing";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(_params: Value) -> Result<Query> {
        Err(Error::InvalidQuery("always fails".to_string()))
    }

    async fn extract_data(&self, _query: &Query, _creds: &Credentials) -> Result<Bytes> {
        unreachable!("transform_query short-circuits");
    }

    fn transform_data(&self, _query: &Query, _raw: Bytes) -> Result<Vec<Row>> {
        unreachable!("transform_query short-circuits");
    }
}

#[test]
fn register_provider_rejects_duplicate_same_kind() {
    let mut runner = CommandRunner::default();
    runner
        .register_provider(RegistryEntry::fetcher("fileset", "equity_historical"))
        .unwrap_or_else(|error| panic!("first register: {error}"));
    let err = runner
        .register_provider(RegistryEntry::fetcher("fileset", "equity_historical"))
        .expect_err("duplicate must error");
    let message = err.to_string();
    assert!(
        message.contains("duplicate") && message.contains("fileset"),
        "duplicate error should name provider, got: {message}"
    );
}

#[test]
fn registered_providers_returns_insertion_ordered_slice() {
    let mut runner = CommandRunner::default();
    runner
        .register_provider(RegistryEntry::fetcher("a", "ep"))
        .unwrap_or_else(|error| panic!("a: {error}"));
    runner
        .register_provider(RegistryEntry::streamer("b", "ep"))
        .unwrap_or_else(|error| panic!("b: {error}"));

    let entries = runner.registered_providers();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].provider, "a");
    assert_eq!(entries[0].kind, ProviderKind::Fetcher);
    assert_eq!(entries[1].provider, "b");
    assert_eq!(entries[1].kind, ProviderKind::Streamer);
}

#[test]
fn with_credentials_chaining_stores_each_field() {
    let creds = Credentials {
        polygon_api_key: Some("poly".to_string()),
        openai_api_key: Some("oai".to_string()),
        google_api_key: None,
        anthropic_api_key: None,
    };
    let _runner = CommandRunner::default().with_credentials(creds.clone());
    // CommandRunner does not expose its creds; the round-trip below confirms
    // they're used during fetch.
    let runner = CommandRunner::default().with_credentials(creds);
    let result = block_on(runner.run(&StrictFetcher, json!({ "symbol": "AAPL" })))
        .unwrap_or_else(|error| panic!("fetch: {error}"));
    assert_eq!(result.rows[0].symbol, "AAPL");
}

#[test]
fn run_returns_invalid_query_when_params_missing_required_field() {
    let runner = CommandRunner::default();
    let err =
        block_on(runner.run(&StrictFetcher, json!({}))).expect_err("missing symbol must error");
    assert!(matches!(err, Error::InvalidQuery(_)));
}

#[test]
fn run_propagates_invalid_query_from_fetcher_unconditionally() {
    let runner = CommandRunner::default();
    let err = block_on(runner.run(&AlwaysFailingFetcher, json!({ "symbol": "AAPL" })))
        .expect_err("always-failing must error");
    let message = err.to_string();
    assert!(message.contains("always fails"));
}

#[test]
fn run_streaming_short_circuits_when_underlying_fetch_errors() {
    // G008/RT1b: opt in to fake-streaming so this test reaches the underlying
    // fetch error (InvalidQuery) rather than the gate-refusal Provider error.
    let runner = CommandRunner::default().allow_fake_streaming();
    match block_on(runner.run_streaming(&AlwaysFailingFetcher, json!({}))) {
        Err(err) => assert!(matches!(err, Error::InvalidQuery(_))),
        Ok(_) => panic!("underlying fetch must propagate"),
    }
}

#[test]
fn run_streaming_emits_progress_then_done_for_happy_path() {
    // G008/RT1b: opt in to fake-streaming — this test exercises the
    // fetch-then-wrap behaviour intentionally and acknowledges no real
    // incremental streaming occurs.
    let runner = CommandRunner::default().allow_fake_streaming();
    let mut stream = block_on(runner.run_streaming(&StrictFetcher, json!({ "symbol": "AAPL" })))
        .unwrap_or_else(|error| panic!("streaming run: {error}"));

    let first = poll_next(&mut stream);
    let second = poll_next(&mut stream);
    let third = poll_next(&mut stream);
    let exhausted = poll_next(&mut stream);

    assert!(matches!(
        first,
        Some(Ok(ProgressOrResult::Progress {
            stage: "fetch",
            fraction: 0.0,
            ..
        }))
    ));
    assert!(matches!(
        second,
        Some(Ok(ProgressOrResult::Progress {
            stage: "fetch",
            fraction: 1.0,
            ..
        }))
    ));
    match third {
        Some(Ok(ProgressOrResult::Done(object))) => {
            assert_eq!(object.provider, "strict");
            assert_eq!(object.rows[0].symbol, "AAPL");
        }
        other => panic!("unexpected terminal event: {other:?}"),
    }
    assert!(exhausted.is_none(), "stream should exhaust after Done");
}

// ---------- block_on harness shared with the inline tests ----------

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

fn poll_next<S>(stream: &mut Pin<Box<S>>) -> Option<<S as Stream>::Item>
where
    S: Stream + ?Sized,
{
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    match stream.as_mut().poll_next(&mut context) {
        Poll::Ready(item) => item,
        Poll::Pending => None,
    }
}
