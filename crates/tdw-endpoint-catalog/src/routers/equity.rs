//! `equity/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{
    CompanyProfile, CorporateAction, EquityHistoricalData, Estimate, OwnershipRecord,
    PricePerformance, QuoteSnapshot,
};

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

/// Keyless SEC candidate for the Form 13F-HR institutional-holdings index
/// (gap-matrix item **L2.6**).
const EQUITY_OWNERSHIP_FORM_13F: &[ProviderCandidate] =
    &[ProviderCandidate::new("sec", "form_13f")];

/// Keyless SEC candidate for fails-to-deliver records (gap-matrix item **L2.6**).
const EQUITY_SHORTS_FAILS_TO_DELIVER: &[ProviderCandidate] =
    &[ProviderCandidate::new("sec", "fails_to_deliver")];
// Keyless Yahoo expansion (gap-matrix item L2.4). Each route's single candidate
// endpoint key matches the Yahoo fetcher's `ENDPOINT` const, which is also the
// runtime fetch/ingest dispatch-table key; a conformance test in tdw-service-api
// keeps these rows and that table in sync.
const EQUITY_PROFILE: &[ProviderCandidate] = &[ProviderCandidate::new("yahoo", "equity_profile")];
const EQUITY_PRICE_QUOTE: &[ProviderCandidate] = &[ProviderCandidate::new("yahoo", "equity_quote")];
const EQUITY_PRICE_PERFORMANCE: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "price_performance")];
const EQUITY_FUNDAMENTAL_DIVIDENDS: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "dividends")];
const EQUITY_OWNERSHIP_SHARE_STATISTICS: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "share_statistics")];
const EQUITY_ESTIMATES_CONSENSUS: &[ProviderCandidate] =
    &[ProviderCandidate::new("yahoo", "analyst_consensus")];

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn model_schema() -> Schema {
    schema_for!(EquityHistoricalData)
}

fn company_profile() -> Schema {
    schema_for!(CompanyProfile)
}

fn price_quote() -> Schema {
    schema_for!(QuoteSnapshot)
}

fn price_performance() -> Schema {
    schema_for!(PricePerformance)
}

fn corporate_action() -> Schema {
    schema_for!(CorporateAction)
}

fn ownership_record() -> Schema {
    schema_for!(OwnershipRecord)
}

fn estimate() -> Schema {
    schema_for!(Estimate)
}

/// The `equity` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            route: "equity/price/historical",
            kind: EndpointKind::Fetch,
            params_schema,
            model: model_schema,
            candidates: EQUITY_PRICE_HISTORICAL,
            bronze_table: Some("raw.equity_historical"),
            doc: "Historical end-of-day OHLCV bars for an equity symbol.",
            chartable: true,
        },
        CatalogEntry {
            route: "equity/ownership/form_13f",
            kind: EndpointKind::Fetch,
            params_schema,
            model: ownership_record,
            candidates: EQUITY_OWNERSHIP_FORM_13F,
            bronze_table: Some("raw.ownership_record"),
            doc: "SEC Form 13F-HR institutional-holdings filing index by CIK (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "equity/shorts/fails_to_deliver",
            kind: EndpointKind::Fetch,
            params_schema,
            model: ownership_record,
            candidates: EQUITY_SHORTS_FAILS_TO_DELIVER,
            bronze_table: Some("raw.ownership_record"),
            doc: "SEC fails-to-deliver records for an equity symbol (keyless).",
            chartable: false,
        },
        CatalogEntry {
            route: "equity/profile",
            kind: EndpointKind::Fetch,
            params_schema,
            model: company_profile,
            candidates: EQUITY_PROFILE,
            bronze_table: Some("raw.company_profile"),
            doc: "Company profile (name, exchange, currency, market cap), Yahoo-backed.",
            chartable: false,
        },
        CatalogEntry {
            route: "equity/price/quote",
            kind: EndpointKind::Fetch,
            params_schema,
            model: price_quote,
            candidates: EQUITY_PRICE_QUOTE,
            bronze_table: Some("raw.price_quote"),
            doc: "Current last-price quote snapshot for an equity symbol, Yahoo-backed.",
            chartable: false,
        },
        CatalogEntry {
            route: "equity/price/performance",
            kind: EndpointKind::Fetch,
            params_schema,
            model: price_performance,
            candidates: EQUITY_PRICE_PERFORMANCE,
            bronze_table: Some("raw.price_performance"),
            doc: "Period total returns (one-day through one-year) for a symbol, Yahoo-backed.",
            chartable: false,
        },
        CatalogEntry {
            route: "equity/fundamental/dividends",
            kind: EndpointKind::Fetch,
            params_schema,
            model: corporate_action,
            candidates: EQUITY_FUNDAMENTAL_DIVIDENDS,
            bronze_table: Some("raw.corporate_action"),
            doc: "Historical cash dividends for an equity symbol, Yahoo-backed.",
            chartable: false,
        },
        CatalogEntry {
            route: "equity/ownership/share_statistics",
            kind: EndpointKind::Fetch,
            params_schema,
            model: ownership_record,
            candidates: EQUITY_OWNERSHIP_SHARE_STATISTICS,
            bronze_table: Some("raw.ownership_record"),
            doc: "Share statistics (float, shares outstanding, ownership), Yahoo-backed.",
            chartable: false,
        },
        CatalogEntry {
            route: "equity/estimates/consensus",
            kind: EndpointKind::Fetch,
            params_schema,
            model: estimate,
            candidates: EQUITY_ESTIMATES_CONSENSUS,
            bronze_table: Some("raw.estimate"),
            doc: "Analyst consensus and price targets for a symbol, Yahoo-backed.",
            chartable: false,
        },
    ]
}
