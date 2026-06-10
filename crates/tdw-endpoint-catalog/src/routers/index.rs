//! `index/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::MarketDataBar;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// Candidate order for `index/price/historical`.
///
/// Bar providers that also serve index symbols; both feature-gated. Matches the
/// legacy resolver order.
const INDEX_PRICE_HISTORICAL: &[ProviderCandidate] = &[
    ProviderCandidate::new("polygon", "aggregates"),
    ProviderCandidate::new("databento", "timeseries"),
];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn model_schema() -> Schema {
    schema_for!(MarketDataBar)
}

/// The `index` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![CatalogEntry {
        route: "index/price/historical",
        kind: EndpointKind::Fetch,
        params_schema,
        model: model_schema,
        candidates: INDEX_PRICE_HISTORICAL,
        bronze_table: Some("raw.market_data_bar"),
        doc: "Historical OHLCV bars for a market index.",
        chartable: true,
    }]
}
