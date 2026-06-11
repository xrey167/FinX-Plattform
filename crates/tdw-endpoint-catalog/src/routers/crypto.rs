//! `crypto/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::MarketDataBar;

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// Candidate order for `crypto/price/historical`.
///
/// All candidates are feature-gated, so this route is unresolvable in an offline
/// build (the consumer's resolver names the candidates in a structured error)
/// and resolvable once any crypto provider is enabled. The two dedicated crypto
/// providers lead (matching the legacy resolver order); the Polygon
/// aggregate-bars fetcher follows as a third candidate (gap-matrix item L2.x
/// polygon), serving crypto pairs via the `X:` ticker prefix (e.g. `X:BTCUSD`).
/// The caller supplies the prefixed ticker, so the existing `aggregates` fetcher
/// and its dispatch binding are reused verbatim.
const CRYPTO_PRICE_HISTORICAL: &[ProviderCandidate] = &[
    ProviderCandidate::new("coingecko", "ohlc"),
    ProviderCandidate::new("ccdata", "crypto_ohlcv"),
    ProviderCandidate::new("polygon", "aggregates"),
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
