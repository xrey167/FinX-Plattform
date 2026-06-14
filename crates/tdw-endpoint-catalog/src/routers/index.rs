//! `index/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{IndexConstituent, Instrument, MarketDataBar, QuoteSnapshot, Sp500Multiple};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

/// CBOE-backed candidate for `index/available` and `index/search`.
///
/// Both routes are served by one keyless CBOE index-directory fetcher: the
/// `index_directory` endpoint reads the public CDN definitions document and
/// emits one [`tdw_domain::Instrument`] per index (the optional `query` param
/// filters for `index/search`). Sharing one `(cboe, index_directory)` candidate
/// across two routes mirrors the shared `polygon/aggregates` candidate.
const INDEX_DIRECTORY: &[ProviderCandidate] = &[ProviderCandidate::new("cboe", "index_directory")];

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

/// FMP-backed candidate for `index/constituents`.
///
/// FMP publishes a per-index constituent list at `/{index}_constituent`
/// (`sp500`/`nasdaq`/`dowjones`); the `index` param selects the index handle.
/// CBOE/TMX constituent feeds are not yet specced from a vendor-published API,
/// so FMP is the single keyed candidate (the established model+one-verifiable-
/// provider pattern, like `currency/search`).
const INDEX_CONSTITUENTS: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "index_constituents")];

/// NASDAQ-backed candidate for `index/sp500_multiples`.
///
/// The keyed NASDAQ Data Link `MULTPL` database serves the long-run S&P 500
/// valuation multiples (Robert Shiller's CAPE PE and friends); the `metric`
/// param selects the dataset. The endpoint key matches
/// `NasdaqHttpSp500MultiplesFetcher::ENDPOINT`.
const INDEX_SP500_MULTIPLES: &[ProviderCandidate] =
    &[ProviderCandidate::new("nasdaq", "sp500_multiples")];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn model_schema() -> Schema {
    schema_for!(MarketDataBar)
}

fn quote_snapshot() -> Schema {
    schema_for!(QuoteSnapshot)
}

fn instrument() -> Schema {
    schema_for!(Instrument)
}

fn index_constituent() -> Schema {
    schema_for!(IndexConstituent)
}

fn sp500_multiple() -> Schema {
    schema_for!(Sp500Multiple)
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
        CatalogEntry {
            route: "index/available",
            kind: EndpointKind::Fetch,
            params_schema,
            model: instrument,
            candidates: INDEX_DIRECTORY,
            bronze_table: Some("raw.instrument"),
            doc: "List available market indices (symbol, name), CBOE-backed (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "index/search",
            kind: EndpointKind::Fetch,
            params_schema,
            model: instrument,
            candidates: INDEX_DIRECTORY,
            bronze_table: Some("raw.instrument"),
            doc: "Search market indices by name or symbol fragment, CBOE-backed (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "index/constituents",
            kind: EndpointKind::Fetch,
            params_schema,
            model: index_constituent,
            candidates: INDEX_CONSTITUENTS,
            bronze_table: Some("raw.index_constituent"),
            doc: "Constituent members of a market index (sp500/nasdaq/dowjones), FMP-backed.",
            chartable: false,
        },
        CatalogEntry {
            route: "index/sp500_multiples",
            kind: EndpointKind::Fetch,
            params_schema,
            model: sp500_multiple,
            candidates: INDEX_SP500_MULTIPLES,
            bronze_table: Some("raw.sp500_multiple"),
            doc: "S&P 500 valuation multiples incl. Shiller CAPE (NASDAQ Data Link MULTPL).",
            chartable: true,
        },
    ]
}
