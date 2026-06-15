//! `crypto/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{Instrument, MarketDataBar};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// FMP-backed candidate for `crypto/search`.
///
/// Reuses the existing FMP `/search` fetcher (the `fmp/search` dispatch binding
/// already registered for `equity/search`): FMP's symbol search spans equities,
/// ETFs, FX, and crypto pairs, so the same keyword search serves crypto-pair
/// discovery. No new fetcher or dispatch binding is added — this is a catalog
/// projection of the existing `(fmp, search)` endpoint onto a second route, the
/// established reuse pattern (mirrors the shared `polygon/aggregates` candidate).
const CRYPTO_SEARCH: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "search")];

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

fn instrument() -> Schema {
    schema_for!(Instrument)
}

/// The `crypto` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "crypto/price/historical",
            kind: EndpointKind::Fetch,
            params_schema,
            model: model_schema,
            candidates: CRYPTO_PRICE_HISTORICAL,
            bronze_table: Some("raw.market_data_bar"),
            doc: "Historical OHLCV bars for a crypto trading pair.",
            chartable: true,
        },
        CatalogEntry {
            route: "crypto/search",
            kind: EndpointKind::Fetch,
            params_schema,
            model: instrument,
            candidates: CRYPTO_SEARCH,
            bronze_table: Some("raw.instrument"),
            doc: "Search available crypto pairs/symbols by name or fragment, FMP-backed.",
            chartable: false,
        },
    ]
}
