//! `index/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{MarketDataBar, QuoteSnapshot};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// Candidate order for `index/price/historical`.
///
/// Bar providers that also serve index symbols; both feature-gated. Matches the
/// legacy resolver order.
const INDEX_PRICE_HISTORICAL: &[ProviderCandidate] = &[
    ProviderCandidate::new("polygon", "aggregates"),
    ProviderCandidate::new("databento", "timeseries"),
];

/// CBOE-backed candidate for `index/snapshots` (delayed US-index quotes). The
/// endpoint key matches `CboeHttpIndexSnapshotFetcher::ENDPOINT`.
const INDEX_SNAPSHOTS: &[ProviderCandidate] = &[ProviderCandidate::new("cboe", "index_snapshots")];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn model_schema() -> Schema {
    schema_for!(MarketDataBar)
}

fn quote_snapshot() -> Schema {
    schema_for!(QuoteSnapshot)
}

/// The `index` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "index/price/historical",
            kind: EndpointKind::Fetch,
            params_schema,
            model: model_schema,
            candidates: INDEX_PRICE_HISTORICAL,
            bronze_table: Some("raw.market_data_bar"),
            doc: "Historical OHLCV bars for a market index.",
            chartable: true,
        },
        CatalogEntry {
            route: "index/snapshots",
            kind: EndpointKind::Fetch,
            params_schema,
            model: quote_snapshot,
            candidates: INDEX_SNAPSHOTS,
            bronze_table: Some("raw.price_quote"),
            doc: "Delayed US-index quote snapshot (price, change), CBOE-backed.",
            chartable: false,
        },
    ]
}
