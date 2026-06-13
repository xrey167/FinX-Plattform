//! `economy/*` catalog routes.
//!
//! The macro cluster (gap-matrix item **L2.3**) is FRED-backed: each route
//! standardizes one `OpenBB` economy command onto a FRED series, normalized to
//! [`tdw_domain::MacroSeries`]. The namespace also carries two keyless
//! non-FRED routes: the Federal Reserve H.6 money-measures table and the Ken
//! French Data Library research factors (`economy/factors/famafrench` →
//! [`tdw_domain::FactorReturn`], OpenBB-parity **P2W6**). The single candidate per route is
//! `(provider="fred", endpoint=<route with '/'→'_'>)` — the exact key the
//! runtime ingest/fetch dispatch table registers under the `provider-fred`
//! feature. Clean-room facts (the route↔series mapping lives in
//! `tdw-provider-fred`'s `ENDPOINTS`); a conformance test in `tdw-service-api`
//! keeps these rows and that table in sync without this crate depending on the
//! provider.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{FactorReturn, FomcDocument, MacroSeries, SeriesSearchResult};

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
/// Keyless Ken French Data Library candidate for the research-factor route
/// (OpenBB-parity **P2W6**). The endpoint key is the route's `'/'→'_'` form,
/// matching `endpoint_key_for_route` and the `famafrench` dispatch binding.
const ECONOMY_FACTORS_FAMAFRENCH: &[ProviderCandidate] = &[ProviderCandidate::new(
    "famafrench",
    "economy_factors_famafrench",
)];
/// IMF SDMX-JSON candidates (OpenBB-parity **P3W3**) — international/cross-country
/// macro series the US-centric FRED cluster does not cover. The endpoint key is
/// the route's `'/'→'_'` form, matching `endpoint_key_for_route` and the
/// `(provider="imf", endpoint=…)` dispatch binding registered under
/// `provider-imf`.
const ECONOMY_IMF_IFS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "imf",
    "economy_imf_international_financial_statistics",
)];
const ECONOMY_IMF_DOT: &[ProviderCandidate] = &[ProviderCandidate::new(
    "imf",
    "economy_imf_direction_of_trade",
)];
const ECONOMY_IMF_BOP: &[ProviderCandidate] = &[ProviderCandidate::new(
    "imf",
    "economy_imf_balance_of_payments",
)];
/// `EconDB` candidate (OpenBB-parity **P3W4**) — standardized cross-country macro
/// series fetched by `EconDB` ticker. The endpoint key is the route's `'/'→'_'`
/// form, matching `endpoint_key_for_route` and the `(provider="econdb",
/// endpoint=…)` dispatch binding registered under `provider-econdb`.
const ECONOMY_ECONDB_SERIES: &[ProviderCandidate] =
    &[ProviderCandidate::new("econdb", "economy_econdb_series")];
/// Keyless Federal Reserve candidate for the full H.6 money-measures table
/// (gap-matrix item **L3.1**) — distinct from the FRED-backed single-series
/// `economy/money_measures/{m1,m2}` routes above.
const ECONOMY_MONEY_MEASURES: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "economy_money_measures",
)];
/// OECD SDMX-JSON candidates (OpenBB-parity **P4W4**) — keyless cross-country
/// macro series the US-centric FRED cluster does not cover. The endpoint key is
/// the route's `'/'→'_'` form, matching `endpoint_key_for_route` and the
/// `(provider="oecd", endpoint=…)` dispatch binding registered under
/// `provider-oecd`.
const ECONOMY_GDP_FORECAST: &[ProviderCandidate] =
    &[ProviderCandidate::new("oecd", "economy_gdp_forecast")];
const ECONOMY_INTEREST_RATES: &[ProviderCandidate] =
    &[ProviderCandidate::new("oecd", "economy_interest_rates")];
const ECONOMY_CLI: &[ProviderCandidate] = &[ProviderCandidate::new(
    "oecd",
    "economy_composite_leading_indicator",
)];
const ECONOMY_HOUSE_PRICE_INDEX: &[ProviderCandidate] =
    &[ProviderCandidate::new("oecd", "economy_house_price_index")];
const ECONOMY_SHARE_PRICE_INDEX: &[ProviderCandidate] =
    &[ProviderCandidate::new("oecd", "economy_share_price_index")];
/// Keyless Federal Reserve candidates (OpenBB-parity **P4W4**) — SOMA holdings
/// and primary-dealer statistics from the public Fed data portals. The endpoint
/// key is the route's `'/'→'_'` form, matching the `(provider="federal_reserve",
/// endpoint=…)` dispatch binding registered under `provider-federal-reserve`.
const ECONOMY_CENTRAL_BANK_HOLDINGS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "economy_central_bank_holdings",
)];
const ECONOMY_PRIMARY_DEALER_POSITIONING: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "economy_primary_dealer_positioning",
)];
const ECONOMY_PRIMARY_DEALER_FAILS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "economy_primary_dealer_fails",
)];
const ECONOMY_FOMC_DOCUMENTS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "economy_fomc_documents",
)];
/// FRED-backed single-series candidates added in OpenBB-parity **P4W4** — survey
/// and price series standardized to [`MacroSeries`], auto-bound by the FRED
/// dispatch loop from `tdw-provider-fred`'s `ENDPOINTS`.
const ECONOMY_RETAIL_PRICES: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "economy_retail_prices")];
const ECONOMY_SURVEY_SLOOS: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "economy_survey_sloos")];
const ECONOMY_SURVEY_CHICAGO: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "economy_survey_economic_conditions_chicago",
)];
const ECONOMY_SURVEY_NY: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "economy_survey_manufacturing_outlook_ny",
)];
const ECONOMY_SURVEY_TEXAS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "economy_survey_manufacturing_outlook_texas",
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

fn factor_return() -> Schema {
    schema_for!(FactorReturn)
}

fn fomc_document() -> Schema {
    schema_for!(FomcDocument)
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
    let mut entries = vec![
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
        macro_entry(
            "economy/imf/international_financial_statistics",
            ECONOMY_IMF_IFS,
            "IMF International Financial Statistics time series, SDMX-JSON macro series.",
        ),
        macro_entry(
            "economy/imf/direction_of_trade",
            ECONOMY_IMF_DOT,
            "IMF Direction of Trade Statistics time series, SDMX-JSON macro series.",
        ),
        macro_entry(
            "economy/imf/balance_of_payments",
            ECONOMY_IMF_BOP,
            "IMF Balance of Payments time series, SDMX-JSON macro series.",
        ),
        macro_entry(
            "economy/econdb/series",
            ECONOMY_ECONDB_SERIES,
            "EconDB economic time series by ticker (standardized cross-country macro series).",
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
        CatalogEntry {
            route: "economy/factors/famafrench",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: factor_return,
            candidates: ECONOMY_FACTORS_FAMAFRENCH,
            bronze_table: Some("raw.factor_return"),
            doc: "Ken French Data Library research factors \
                  (3-factor / 5-factor / momentum, daily or monthly; keyless).",
            chartable: true,
        },
    ];
    entries.extend(p4w4_entries());
    entries
}

/// The `economy` namespace's OpenBB-parity **P4W4** breadth additions: the OECD
/// SDMX-JSON cross-country macro cluster, the FRED single-series survey / price
/// breadth, and the keyless Federal Reserve SOMA / primary-dealer routes. Split
/// from [`entries`] so neither function trips the `too_many_lines` lint.
fn p4w4_entries() -> Vec<CatalogEntry> {
    vec![
        // -- OECD SDMX-JSON keyless cross-country macro -----------------------
        macro_entry(
            "economy/gdp/forecast",
            ECONOMY_GDP_FORECAST,
            "OECD Economic Outlook real GDP forecast (SDMX-JSON, keyless).",
        ),
        macro_entry(
            "economy/interest_rates",
            ECONOMY_INTEREST_RATES,
            "OECD Main Economic Indicators interest rates (SDMX-JSON, keyless).",
        ),
        macro_entry(
            "economy/composite_leading_indicator",
            ECONOMY_CLI,
            "OECD Composite Leading Indicator (SDMX-JSON, keyless).",
        ),
        macro_entry(
            "economy/house_price_index",
            ECONOMY_HOUSE_PRICE_INDEX,
            "OECD House Price Index (SDMX-JSON, keyless).",
        ),
        macro_entry(
            "economy/share_price_index",
            ECONOMY_SHARE_PRICE_INDEX,
            "OECD share price index (SDMX-JSON, keyless).",
        ),
        // -- FRED single-series survey / price breadth ------------------------
        macro_entry(
            "economy/retail_prices",
            ECONOMY_RETAIL_PRICES,
            "Advance retail and food services sales, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/survey/sloos",
            ECONOMY_SURVEY_SLOOS,
            "Senior Loan Officer Opinion Survey net tightening (C&I loans), FRED-backed.",
        ),
        macro_entry(
            "economy/survey/economic_conditions_chicago",
            ECONOMY_SURVEY_CHICAGO,
            "Chicago Fed National Activity Index, FRED-backed macro series.",
        ),
        macro_entry(
            "economy/survey/manufacturing_outlook_ny",
            ECONOMY_SURVEY_NY,
            "Empire State Manufacturing Survey general business conditions, FRED-backed.",
        ),
        macro_entry(
            "economy/survey/manufacturing_outlook_texas",
            ECONOMY_SURVEY_TEXAS,
            "Dallas Fed Texas Manufacturing Outlook general business activity, FRED-backed.",
        ),
        // -- Federal Reserve keyless SOMA / primary-dealer breadth ------------
        macro_entry(
            "economy/central_bank_holdings",
            ECONOMY_CENTRAL_BANK_HOLDINGS,
            "Federal Reserve SOMA securities holdings (keyless).",
        ),
        macro_entry(
            "economy/primary_dealer_positioning",
            ECONOMY_PRIMARY_DEALER_POSITIONING,
            "FRBNY primary-dealer net positioning (keyless).",
        ),
        macro_entry(
            "economy/primary_dealer_fails",
            ECONOMY_PRIMARY_DEALER_FAILS,
            "FRBNY primary-dealer settlement fails (keyless).",
        ),
        CatalogEntry {
            route: "economy/fomc_documents",
            kind: EndpointKind::Fetch,
            params_schema: standard_params,
            model: fomc_document,
            candidates: ECONOMY_FOMC_DOCUMENTS,
            bronze_table: Some("raw.fomc_document"),
            doc: "FOMC meeting documents index (Federal Reserve, keyless).",
            chartable: false,
        },
    ]
}
