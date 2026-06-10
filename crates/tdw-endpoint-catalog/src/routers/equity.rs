//! `equity/*` catalog routes.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{EquityHistoricalData, OwnershipRecord};

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

fn params_schema() -> Schema {
    schema_for!(StandardParams)
}

fn model_schema() -> Schema {
    schema_for!(EquityHistoricalData)
}

fn ownership_record() -> Schema {
    schema_for!(OwnershipRecord)
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
    ]
}
