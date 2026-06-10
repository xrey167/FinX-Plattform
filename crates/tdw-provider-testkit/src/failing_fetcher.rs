//! Deterministic test fetchers for runtime-fallback conformance tests.
//!
//! [`FailingEquityHistoricalFetcher`] always fails with a retryable
//! [`tdw_core::Error::Provider`] (a provider-side error, not a validation
//! error), so a fallback-aware dispatcher must skip it and try the next
//! candidate. [`StubEquityHistoricalFetcher`] always succeeds with one fixture
//! row, standing in for the candidate the fallback lands on. Both are keyed to
//! real `equity/price/historical` catalog candidates so they slot into the
//! ingest dispatch table without inventing routes.

use async_trait::async_trait;
use bytes::Bytes;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::EquityHistoricalData;

/// Query params for the test equity fetchers: just the symbol.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TestEquityQuery {
    /// Equity symbol to (pretend to) fetch.
    pub symbol: String,
}

fn parse_query(params: &Value) -> Result<TestEquityQuery> {
    let symbol = params
        .get("symbol")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidQuery("missing symbol".to_string()))?;
    Ok(TestEquityQuery {
        symbol: symbol.to_string(),
    })
}

/// A fetcher that always fails with a retryable provider-side error.
///
/// Keyed to the `fmp`/`equity_historical` candidate of
/// `equity/price/historical`. The failure is [`Error::Provider`] (retryable),
/// never [`Error::InvalidQuery`] (fail-fast), so a fallback-aware dispatcher
/// proceeds to the next candidate instead of surfacing it.
#[derive(Clone, Debug, Default)]
pub struct FailingEquityHistoricalFetcher;

impl FailingEquityHistoricalFetcher {
    /// Registry entry for this fetcher (`fmp`/`equity_historical`).
    #[must_use]
    pub const fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<TestEquityQuery, EquityHistoricalData> for FailingEquityHistoricalFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<TestEquityQuery> {
        parse_query(&params)
    }

    async fn extract_data(&self, _query: &TestEquityQuery, _creds: &Credentials) -> Result<Bytes> {
        Err(Error::Provider(
            "simulated upstream 503 from fmp (retryable provider error)".to_string(),
        ))
    }

    fn transform_data(
        &self,
        _query: &TestEquityQuery,
        _raw: Bytes,
    ) -> Result<Vec<EquityHistoricalData>> {
        // Unreachable: extract_data always errors first.
        Ok(Vec::new())
    }
}

/// A fetcher that always succeeds with one fixture row.
///
/// Keyed to the `akshare`/`hist` candidate of `equity/price/historical` — the
/// last candidate in declaration order — so a fallback test can register a
/// failing earlier candidate plus this working later one and assert the dispatch
/// lands here.
#[derive(Clone, Debug, Default)]
pub struct StubEquityHistoricalFetcher;

impl StubEquityHistoricalFetcher {
    /// Registry entry for this fetcher (`akshare`/`hist`).
    #[must_use]
    pub const fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<TestEquityQuery, EquityHistoricalData> for StubEquityHistoricalFetcher {
    const PROVIDER: &'static str = "akshare";
    const ENDPOINT: &'static str = "hist";

    fn transform_query(params: Value) -> Result<TestEquityQuery> {
        parse_query(&params)
    }

    async fn extract_data(&self, query: &TestEquityQuery, _creds: &Credentials) -> Result<Bytes> {
        let rows = vec![EquityHistoricalData {
            symbol: query.symbol.clone(),
            date: "2026-05-21".to_string(),
            open: 100.0,
            high: 102.0,
            low: 99.0,
            close: 101.0,
            volume: 10_000,
        }];
        serde_json::to_vec(&rows)
            .map(Bytes::from)
            .map_err(|error| Error::Provider(error.to_string()))
    }

    fn transform_data(
        &self,
        _query: &TestEquityQuery,
        raw: Bytes,
    ) -> Result<Vec<EquityHistoricalData>> {
        serde_json::from_slice(&raw).map_err(|error| Error::Provider(error.to_string()))
    }
}
