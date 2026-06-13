//! `etf/*` catalog routes (gap-matrix item **L2.6**, the ETF cluster).
//!
//! ETF holdings are served keylessly from SEC N-PORT (`NPORT-P`) portfolio
//! disclosures. The remaining cluster (search, info, historical, sectors,
//! countries, price performance, equity exposure) is served by FMP's documented
//! ETF endpoints (free key); `etf/info` and `etf/historical` additionally carry a
//! keyless Yahoo candidate, and `etf/nport_disclosure` is served by the keyless
//! SEC submissions index. A per-provider conformance test in `tdw-service-api`
//! keeps each route's candidate endpoint key and the provider's dispatch table in
//! sync.
//!
//! `etf/discovery/{active,gainers,losers}` are intentionally not wired here: their
//! only `OpenBB` provider is WSJ (scrape-only, no public API), so they are deferred
//! by documented business decision.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{
    CompanyFiling, EquityHistoricalData, EtfCountryWeight, EtfEquityExposure, EtfHolding, EtfInfo,
    EtfSectorWeight, Instrument, PricePerformance,
};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

const ETF_HOLDINGS: &[ProviderCandidate] = &[ProviderCandidate::new("sec", "etf_holdings")];

// ETF cluster breadth (openbb-parity P4W3). Each FMP route is its own FMP fetcher
// keyed by its `ENDPOINT` const; `etf/info` and `etf/historical` lead with a
// keyless candidate (Yahoo info / the offline-then-Yahoo historical chain). The
// SEC N-PORT disclosure index is keyless. etf/historical reuses the shared OHLCV
// model and the equity-historical dispatch keys (an ETF ticker resolves like any
// symbol through both providers' historical fetchers).
const ETF_SEARCH: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "etf_search")];
const ETF_INFO: &[ProviderCandidate] = &[
    ProviderCandidate::new("yahoo", "etf_info"),
    ProviderCandidate::new("fmp", "etf_info"),
];
const ETF_HISTORICAL: &[ProviderCandidate] = &[
    ProviderCandidate::new("yahoo", "equity_historical"),
    ProviderCandidate::new("fmp", "equity_historical"),
];
const ETF_SECTORS: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "etf_sectors")];
const ETF_COUNTRIES: &[ProviderCandidate] = &[ProviderCandidate::new("fmp", "etf_countries")];
const ETF_PRICE_PERFORMANCE: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "etf_price_performance")];
const ETF_EQUITY_EXPOSURE: &[ProviderCandidate] =
    &[ProviderCandidate::new("fmp", "etf_equity_exposure")];
const ETF_NPORT_DISCLOSURE: &[ProviderCandidate] =
    &[ProviderCandidate::new("sec", "nport_disclosure")];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn etf_holding() -> Schema {
    schema_for!(EtfHolding)
}

fn instrument() -> Schema {
    schema_for!(Instrument)
}

fn etf_info() -> Schema {
    schema_for!(EtfInfo)
}

fn equity_historical() -> Schema {
    schema_for!(EquityHistoricalData)
}

fn etf_sector_weight() -> Schema {
    schema_for!(EtfSectorWeight)
}

fn etf_country_weight() -> Schema {
    schema_for!(EtfCountryWeight)
}

fn price_performance() -> Schema {
    schema_for!(PricePerformance)
}

fn etf_equity_exposure() -> Schema {
    schema_for!(EtfEquityExposure)
}

fn company_filing() -> Schema {
    schema_for!(CompanyFiling)
}

/// One non-chartable single-model `etf/*` Fetch entry.
fn flat_entry(
    route: &'static str,
    model: fn() -> Schema,
    candidates: &'static [ProviderCandidate],
    bronze_table: &'static str,
    doc: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        route,
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model,
        candidates,
        bronze_table: Some(bronze_table),
        doc,
        chartable: false,
    }
}

/// The `etf` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        flat_entry(
            "etf/holdings",
            etf_holding,
            ETF_HOLDINGS,
            "raw.etf_holding",
            "ETF constituent holdings from SEC N-PORT (NPORT-P) disclosures (keyless).",
        ),
        flat_entry(
            "etf/search",
            instrument,
            ETF_SEARCH,
            "raw.instrument",
            "Search ETFs by name or symbol fragment, FMP-backed.",
        ),
        flat_entry(
            "etf/info",
            etf_info,
            ETF_INFO,
            "raw.etf_info",
            "ETF profile / reference information; Yahoo (keyless) then FMP candidate.",
        ),
        CatalogEntry {
            route: "etf/historical",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: equity_historical,
            candidates: ETF_HISTORICAL,
            bronze_table: Some("raw.equity_historical"),
            doc: "Historical end-of-day OHLCV bars for an ETF symbol; Yahoo (keyless) then FMP \
                  candidate.",
            chartable: true,
        },
        flat_entry(
            "etf/sectors",
            etf_sector_weight,
            ETF_SECTORS,
            "raw.etf_sector_weight",
            "Sector allocation of an ETF, FMP-backed.",
        ),
        flat_entry(
            "etf/countries",
            etf_country_weight,
            ETF_COUNTRIES,
            "raw.etf_country_weight",
            "Country allocation of an ETF, FMP-backed.",
        ),
        flat_entry(
            "etf/price_performance",
            price_performance,
            ETF_PRICE_PERFORMANCE,
            "raw.price_performance",
            "ETF period total returns (one-day through one-year), FMP-backed.",
        ),
        flat_entry(
            "etf/equity_exposure",
            etf_equity_exposure,
            ETF_EQUITY_EXPOSURE,
            "raw.etf_equity_exposure",
            "ETFs holding a given equity (reverse exposure lookup), FMP-backed.",
        ),
        flat_entry(
            "etf/nport_disclosure",
            company_filing,
            ETF_NPORT_DISCLOSURE,
            "raw.company_filing",
            "Fund N-PORT portfolio-disclosure filing index for a filer, SEC-backed (keyless).",
        ),
    ]
}
