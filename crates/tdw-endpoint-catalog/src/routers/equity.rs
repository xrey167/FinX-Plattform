//! `equity/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::EquityHistoricalData;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// Candidate order for `equity/price/historical`.
///
/// The two offline fixture fetchers lead so an unkeyed default build resolves
/// network-free; the keyed HTTP providers follow. This is byte-for-byte the
/// order the legacy `provider_resolve::logical_endpoint_table` declared.
const EQUITY_PRICE_HISTORICAL: &[ProviderCandidate] = &[
    ProviderCandidate::new("fileset", "equity_historical"),
    ProviderCandidate::new("yahoo", "equity_historical"),
    ProviderCandidate::new("fmp", "equity_historical"),
    ProviderCandidate::new("tiingo", "historical"),
    ProviderCandidate::new("polygon", "aggregates"),
    ProviderCandidate::new("alpaca", "stock_bars"),
    ProviderCandidate::new("alpha_vantage", "market_data"),
    ProviderCandidate::new("databento", "timeseries"),
    ProviderCandidate::new("akshare", "hist"),
];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn model_schema() -> Schema {
    schema_for!(EquityHistoricalData)
}

/// The `equity` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![CatalogEntry {
        route: "equity/price/historical",
        kind: EndpointKind::Fetch,
        params_schema,
        model: model_schema,
        candidates: EQUITY_PRICE_HISTORICAL,
        bronze_table: Some("raw.equity_historical"),
        doc: "Historical end-of-day OHLCV bars for an equity symbol.",
        chartable: true,
    }]
}
