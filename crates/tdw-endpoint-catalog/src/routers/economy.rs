//! `economy/*` catalog routes.
//!
//! The macro cluster (gap-matrix item **L2.3**) is FRED-backed: each route
//! standardizes one `OpenBB` economy command onto a FRED series, normalized to
//! [`tdw_domain::MacroSeries`]. The single candidate per route is
//! `(provider="fred", endpoint=<route with '/'→'_'>)` — the exact key the
//! runtime ingest/fetch dispatch table registers under the `provider-fred`
//! feature. Clean-room facts (the route↔series mapping lives in
//! `tdw-provider-fred`'s `ENDPOINTS`); a conformance test in `tdw-service-api`
//! keeps these rows and that table in sync without this crate depending on the
//! provider.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{MacroSeries, SeriesSearchResult};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

// One FRED candidate per macro command. The endpoint key is the route with
// each `'/'` replaced by `'_'`, matching `endpoint_key_for_route`.
const ECONOMY_CPI: &[ProviderCandidate] = &[ProviderCandidate::new("fred", "economy_cpi")];
const ECONOMY_PCE: &[ProviderCandidate] = &[ProviderCandidate::new("fred", "economy_pce")];
const ECONOMY_GDP_REAL: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "economy_gdp_real")];
const ECONOMY_GDP_NOMINAL: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "economy_gdp_nominal")];
const ECONOMY_UNEMPLOYMENT: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "economy_unemployment")];
const ECONOMY_M1: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "economy_money_measures_m1")];
const ECONOMY_M2: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "economy_money_measures_m2")];
const ECONOMY_NONFARM: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "economy_survey_nonfarm_payrolls",
)];
const ECONOMY_UMICH: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "economy_survey_university_of_michigan",
)];
const ECONOMY_INFLATION_EXPECTATIONS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "economy_survey_inflation_expectations",
)];
const ECONOMY_FRED_SEARCH: &[ProviderCandidate] = &[ProviderCandidate::new("fred", "fred_search")];
/// Keyless Federal Reserve candidate for the full H.6 money-measures table
/// (gap-matrix item **L3.1**) — distinct from the FRED-backed single-series
/// `economy/money_measures/{m1,m2}` routes above.
const ECONOMY_MONEY_MEASURES: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "economy_money_measures",
)];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn macro_series() -> Schema {
    schema_for!(MacroSeries)
}

fn series_search_result() -> Schema {
    schema_for!(SeriesSearchResult)
}

/// One FRED macro-series catalog entry (`economy/*` → [`MacroSeries`]).
fn macro_entry(
    route: &'static str,
    candidates: &'static [ProviderCandidate],
    doc: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        route,
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model: macro_series,
        candidates,
        bronze_table: Some("raw.macro_series"),
        doc,
        chartable: true,
    }
}

/// The `economy` namespace's catalog entries, in declaration order.
pub fn entries() -> Vec<CatalogEntry> {
    vec![
        macro_entry(
            "economy/cpi",
            ECONOMY_CPI,
            "Consumer Price Index (headline CPI-U), FRED-backed macro series.",
        ),
        macro_entry(
            "economy/pce",
            ECONOMY_PCE,
            "Personal Consumption Expenditures price index, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/gdp/real",
            ECONOMY_GDP_REAL,
            "Real Gross Domestic Product, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/gdp/nominal",
            ECONOMY_GDP_NOMINAL,
            "Nominal Gross Domestic Product, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/unemployment",
            ECONOMY_UNEMPLOYMENT,
            "Unemployment rate, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/money_measures/m1",
            ECONOMY_M1,
            "M1 money stock, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/money_measures/m2",
            ECONOMY_M2,
            "M2 money stock, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/money_measures",
            ECONOMY_MONEY_MEASURES,
            "Full H.6 money stock measures table (Federal Reserve, keyless).",
        ),
        macro_entry(
            "economy/survey/nonfarm_payrolls",
            ECONOMY_NONFARM,
            "Total nonfarm payroll employment, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/survey/university_of_michigan",
            ECONOMY_UMICH,
            "University of Michigan consumer sentiment, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/survey/inflation_expectations",
            ECONOMY_INFLATION_EXPECTATIONS,
            "University of Michigan inflation expectations, FRED-backed macro series.",
        ),
        CatalogEntry {
            route: "economy/fred_search",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: series_search_result,
            candidates: ECONOMY_FRED_SEARCH,
            bronze_table: Some("raw.series_search_result"),
            doc: "Search FRED series metadata by free text (discovery, no observations).",
            chartable: false,
        },
    ]
}
