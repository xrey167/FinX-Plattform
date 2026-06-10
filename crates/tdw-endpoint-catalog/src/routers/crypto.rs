//! `crypto/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::MarketDataBar;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// Candidate order for `crypto/price/historical`.
///
/// Both candidates are feature-gated, so this route is unresolvable in an
/// offline build (the consumer's resolver names the candidates in a structured
/// error) and resolvable once either crypto provider is enabled. Matches the
/// legacy resolver order.
const CRYPTO_PRICE_HISTORICAL: &[ProviderCandidate] = &[
    ProviderCandidate::new("coingecko", "ohlc"),
    ProviderCandidate::new("ccdata", "crypto_ohlcv"),
];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn model_schema() -> Schema {
    schema_for!(MarketDataBar)
}

/// The `crypto` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![CatalogEntry {
        route: "crypto/price/historical",
        kind: EndpointKind::Fetch,
        params_schema,
        model: model_schema,
        candidates: CRYPTO_PRICE_HISTORICAL,
        bronze_table: Some("raw.market_data_bar"),
        doc: "Historical OHLCV bars for a crypto trading pair.",
        chartable: true,
    }]
}
