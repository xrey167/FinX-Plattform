//! Offline `tdw-core` example: a toy [`Fetcher`] driven over fixture bytes,
//! plus registration into a [`ProviderRegistry`].
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-core --example tdw_core_basic
//! ```
//!
//! No network, no database, no async runtime crate: the provided `fetch`
//! orchestration is driven by a tiny `std`-only executor so the example stays
//! dependency-free.

#![forbid(unsafe_code)]

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use async_trait::async_trait;
use bytes::Bytes;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tdw_core::{
    Credentials, Error, Fetcher, ProviderKind, ProviderRegistry, RegistryEntry, Result,
};

/// A typed query: the symbol to look up.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct QuoteQuery {
    symbol: String,
}

/// A typed row: one quote.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
struct Quote {
    symbol: String,
    price: f64,
}

/// A toy provider. In a real `tdw-provider-*` crate `extract_data` would do
/// network or file I/O; here it returns deterministic fixture bytes.
struct DemoFetcher;

impl DemoFetcher {
    /// The registry-entry convention every provider exposes.
    const fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(
            <Self as Fetcher<QuoteQuery, Quote>>::PROVIDER,
            <Self as Fetcher<QuoteQuery, Quote>>::ENDPOINT,
        )
    }
}

#[async_trait]
impl Fetcher<QuoteQuery, Quote> for DemoFetcher {
    const PROVIDER: &'static str = "demo";
    const ENDPOINT: &'static str = "equity_quote";

    // Associated (no `&self`): validate + normalize untyped JSON into `QuoteQuery`.
    fn transform_query(params: Value) -> Result<QuoteQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("missing symbol".to_string()))?;
        Ok(QuoteQuery {
            symbol: symbol.trim().to_ascii_uppercase(),
        })
    }

    // The only I/O stage: returns opaque fixture bytes.
    async fn extract_data(&self, query: &QuoteQuery, _creds: &Credentials) -> Result<Bytes> {
        let body = json!([{ "symbol": query.symbol, "price": 101.5 }]);
        serde_json::to_vec(&body)
            .map(Bytes::from)
            .map_err(|error| Error::Provider(error.to_string()))
    }

    // Parse raw bytes into typed rows.
    fn transform_data(&self, _query: &QuoteQuery, raw: Bytes) -> Result<Vec<Quote>> {
        serde_json::from_slice(&raw).map_err(|error| Error::Provider(error.to_string()))
    }
}

fn main() {
    // Register the provider and prove duplicate-registration is rejected.
    let mut registry = ProviderRegistry::default();
    registry
        .register(DemoFetcher::registry_entry())
        .expect("first registration succeeds");
    assert!(registry.contains("demo", "equity_quote", ProviderKind::Fetcher));
    assert!(
        registry.register(DemoFetcher::registry_entry()).is_err(),
        "duplicate (provider, endpoint, kind) is rejected"
    );

    // Drive the provided `fetch` orchestration (transform -> extract -> transform).
    let object = block_on(DemoFetcher.fetch(json!({ "symbol": "aapl" }), &Credentials::default()))
        .expect("fetch should succeed over fixture bytes");

    assert_eq!(object.provider, "demo");
    assert_eq!(object.endpoint, "equity_quote");
    assert_eq!(object.rows.len(), 1);
    assert_eq!(object.rows[0].symbol, "AAPL");

    println!(
        "fetched {} row(s) from {}/{}: {:?}",
        object.rows.len(),
        object.provider,
        object.endpoint,
        object.rows
    );
}

/// Minimal `std`-only executor: polls a future to completion on the current
/// thread with a no-op waker. The example's future never actually suspends
/// (the fixture is ready immediately), so a single poll completes it.
fn block_on<F: Future>(future: F) -> F::Output {
    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}
