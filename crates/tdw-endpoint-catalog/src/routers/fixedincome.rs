//! `fixedincome/*` catalog routes.
//!
//! The rate / spread / index cluster (gap-matrix item **L2.3**) is FRED-backed:
//! each route standardizes one `OpenBB` fixedincome command onto a FRED series,
//! normalized to [`tdw_domain::RateObservation`]. The single candidate per route
//! is `(provider="fred", endpoint=<route with '/'→'_'>)` — the exact key the
//! runtime dispatch table registers under the `provider-fred` feature.
//!
//! The aggregate `fixedincome/government/yield_curve` route is also FRED-backed
//! but emits [`tdw_domain::YieldCurvePoint`] by merging the four Treasury
//! constant-maturity series (3m / 2y / 10y / 30y) inside one fetcher.

use schemars::{Schema, schema_for};
use tdw_core::query_params::StandardParams;
use tdw_domain::{MacroSeries, RateObservation, TreasuryAuction, TreasuryPrice, YieldCurvePoint};

use crate::{CatalogEntry, EndpointKind, ProviderCandidate};

// One FRED candidate per rate command. The endpoint key is the route with each
// `'/'` replaced by `'_'`, matching `endpoint_key_for_route`.
const RATE_SOFR: &[ProviderCandidate] = &[ProviderCandidate::new("fred", "fixedincome_rate_sofr")];
const RATE_EFFR: &[ProviderCandidate] = &[ProviderCandidate::new("fred", "fixedincome_rate_effr")];
const RATE_ESTR: &[ProviderCandidate] = &[ProviderCandidate::new("fred", "fixedincome_rate_estr")];
const RATE_SONIA: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "fixedincome_rate_sonia")];
const RATE_ECB: &[ProviderCandidate] = &[ProviderCandidate::new("fred", "fixedincome_rate_ecb")];
const RATE_IORB: &[ProviderCandidate] = &[ProviderCandidate::new("fred", "fixedincome_rate_iorb")];
const RATE_DPCREDIT: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "fixedincome_rate_dpcredit")];
const RATE_OBFR: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_rate_overnight_bank_funding",
)];
const GOV_TREASURY_3M: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_treasury_rates_3m",
)];
const GOV_TREASURY_2Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_treasury_rates_2y",
)];
const GOV_TREASURY_10Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_treasury_rates_10y",
)];
const GOV_TREASURY_30Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_treasury_rates_30y",
)];
const GOV_TIPS_10Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_tips_yields_10y",
)];
const RATE_AMERIBOR: &[ProviderCandidate] =
    &[ProviderCandidate::new("fred", "fixedincome_rate_ameribor")];
const RATE_EFFR_FORECAST: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_rate_effr_forecast",
)];
const RATE_EFFR_FORECAST_LR: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_rate_effr_forecast_long_run",
)];
const GOV_SVENSSON_2Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_svensson_yield_curve_2y",
)];
const GOV_SVENSSON_5Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_svensson_yield_curve_5y",
)];
const GOV_SVENSSON_10Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_svensson_yield_curve_10y",
)];
const SPREAD_TCM_EFFR_1Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_spreads_tcm_effr_1y",
)];
const SPREAD_TCM_EFFR_10Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_spreads_tcm_effr_10y",
)];
const CORP_HQM_2Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_corporate_hqm_2y",
)];
const CORP_HQM_5Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_corporate_hqm_5y",
)];
const CORP_HQM_30Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_corporate_hqm_30y",
)];
const SPREAD_TCM_10Y2Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_spreads_tcm_10y2y",
)];
const SPREAD_TCM_10Y3M: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_spreads_tcm_10y3m",
)];
const SPREAD_TREASURY_EFFR_3M: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_spreads_treasury_effr_3m",
)];
const CORP_SPOT_10Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_corporate_spot_rates_10y",
)];
const CORP_CP_90D: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_corporate_commercial_paper_90d",
)];
const BOND_INDICES_HY: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_bond_indices_us_corporate_hy",
)];
const MORTGAGE_30Y: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_mortgage_indices_30y_fixed",
)];
const GOV_YIELD_CURVE: &[ProviderCandidate] = &[ProviderCandidate::new(
    "fred",
    "fixedincome_government_yield_curve",
)];

/// Keyless US-Treasury `FiscalData` candidates (gap-matrix item **L3.2**) and the
/// keyless Federal Reserve primary-dealer candidate (gap-matrix item **L3.1**).
const GOV_TREASURY_AUCTIONS: &[ProviderCandidate] =
    &[ProviderCandidate::new("government_us", "treasury_auctions")];
const GOV_TREASURY_PRICES: &[ProviderCandidate] =
    &[ProviderCandidate::new("government_us", "treasury_prices")];
const GOV_DEALER_STATS: &[ProviderCandidate] = &[ProviderCandidate::new(
    "federal_reserve",
    "fixedincome_government_dealer_stats",
)];

fn standard_params() -> Schema {
    schema_for!(StandardParams)
}

fn rate_observation() -> Schema {
    schema_for!(RateObservation)
}

fn yield_curve_point() -> Schema {
    schema_for!(YieldCurvePoint)
}

fn treasury_auction() -> Schema {
    schema_for!(TreasuryAuction)
}

fn treasury_price() -> Schema {
    schema_for!(TreasuryPrice)
}

fn macro_series() -> Schema {
    schema_for!(MacroSeries)
}

/// One FRED rate-observation catalog entry (`fixedincome/*` → [`RateObservation`]).
fn rate_entry(
    route: &'static str,
    candidates: &'static [ProviderCandidate],
    doc: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        route,
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model: rate_observation,
        candidates,
        bronze_table: Some("raw.rate_observation"),
        doc,
        chartable: true,
    }
}

/// All single-series rate routes in declaration order: `(route, candidates,
/// doc)`. Each becomes a [`rate_entry`]. Kept as a flat table so the route list
/// is one append, not a new wiring point, and `entries()` stays small.
type RateRow = (&'static str, &'static [ProviderCandidate], &'static str);

const RATE_ROWS: &[RateRow] = &[
    (
        "fixedincome/rate/sofr",
        RATE_SOFR,
        "Secured Overnight Financing Rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/effr",
        RATE_EFFR,
        "Effective Federal Funds Rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/estr",
        RATE_ESTR,
        "Euro Short-Term Rate (\u{20ac}STR), FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/sonia",
        RATE_SONIA,
        "Sterling Overnight Index Average (SONIA), FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/ecb",
        RATE_ECB,
        "ECB deposit facility rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/iorb",
        RATE_IORB,
        "Interest Rate on Reserve Balances, FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/dpcredit",
        RATE_DPCREDIT,
        "Discount window primary credit rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/overnight_bank_funding",
        RATE_OBFR,
        "Overnight Bank Funding Rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/government/treasury_rates/3m",
        GOV_TREASURY_3M,
        "3-Month Treasury constant-maturity yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/government/treasury_rates/2y",
        GOV_TREASURY_2Y,
        "2-Year Treasury constant-maturity yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/government/treasury_rates/10y",
        GOV_TREASURY_10Y,
        "10-Year Treasury constant-maturity yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/government/treasury_rates/30y",
        GOV_TREASURY_30Y,
        "30-Year Treasury constant-maturity yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/government/tips_yields/10y",
        GOV_TIPS_10Y,
        "10-Year TIPS constant-maturity yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/government/svensson_yield_curve/2y",
        GOV_SVENSSON_2Y,
        "2-Year Svensson (GSW) fitted zero-coupon yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/government/svensson_yield_curve/5y",
        GOV_SVENSSON_5Y,
        "5-Year Svensson (GSW) fitted zero-coupon yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/government/svensson_yield_curve/10y",
        GOV_SVENSSON_10Y,
        "10-Year Svensson (GSW) fitted zero-coupon yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/ameribor",
        RATE_AMERIBOR,
        "Overnight Unsecured AMERIBOR benchmark rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/effr_forecast",
        RATE_EFFR_FORECAST,
        "FOMC median fed-funds-rate projection, FRED-backed rate observation.",
    ),
    (
        "fixedincome/rate/effr_forecast/long_run",
        RATE_EFFR_FORECAST_LR,
        "FOMC longer-run median fed-funds-rate projection, FRED-backed rate observation.",
    ),
    (
        "fixedincome/spreads/tcm/10y2y",
        SPREAD_TCM_10Y2Y,
        "10-Year minus 2-Year Treasury constant-maturity spread, FRED-backed.",
    ),
    (
        "fixedincome/spreads/tcm/10y3m",
        SPREAD_TCM_10Y3M,
        "10-Year minus 3-Month Treasury constant-maturity spread, FRED-backed.",
    ),
    (
        "fixedincome/spreads/treasury_effr/3m",
        SPREAD_TREASURY_EFFR_3M,
        "3-Month Treasury minus Federal Funds Rate spread, FRED-backed.",
    ),
    (
        "fixedincome/spreads/tcm_effr/1y",
        SPREAD_TCM_EFFR_1Y,
        "1-Year Treasury constant-maturity minus Federal Funds Rate spread, FRED-backed.",
    ),
    (
        "fixedincome/spreads/tcm_effr/10y",
        SPREAD_TCM_EFFR_10Y,
        "10-Year Treasury constant-maturity minus Federal Funds Rate spread, FRED-backed.",
    ),
    (
        "fixedincome/corporate/spot_rates/10y",
        CORP_SPOT_10Y,
        "10-Year HQM corporate bond spot rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/corporate/hqm/2y",
        CORP_HQM_2Y,
        "2-Year HQM corporate bond yield-curve spot rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/corporate/hqm/5y",
        CORP_HQM_5Y,
        "5-Year HQM corporate bond yield-curve spot rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/corporate/hqm/30y",
        CORP_HQM_30Y,
        "30-Year HQM corporate bond yield-curve spot rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/corporate/commercial_paper/90d",
        CORP_CP_90D,
        "90-Day AA nonfinancial commercial paper rate, FRED-backed rate observation.",
    ),
    (
        "fixedincome/bond_indices/us_corporate_hy",
        BOND_INDICES_HY,
        "ICE BofA US High Yield index effective yield, FRED-backed rate observation.",
    ),
    (
        "fixedincome/mortgage_indices/30y_fixed",
        MORTGAGE_30Y,
        "30-Year fixed-rate mortgage average, FRED-backed rate observation.",
    ),
];

/// The `fixedincome` namespace's catalog entries, in declaration order: the
/// single-series rate cluster followed by the aggregate yield curve.
pub fn entries() -> Vec<CatalogEntry> {
    let mut entries: Vec<CatalogEntry> = RATE_ROWS
        .iter()
        .map(|&(route, candidates, doc)| rate_entry(route, candidates, doc))
        .collect();
    entries.push(CatalogEntry {
        route: "fixedincome/government/yield_curve",
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model: yield_curve_point,
        candidates: GOV_YIELD_CURVE,
        bronze_table: Some("raw.yield_curve_point"),
        doc: "US Treasury yield curve (3m/2y/10y/30y), aggregated from FRED \
              constant-maturity series.",
        chartable: true,
    });
    // Keyless government wave (L3.1 / L3.2): Treasury auctions + prices from US
    // Treasury FiscalData, and primary-dealer stats from the Federal Reserve.
    entries.push(CatalogEntry {
        route: "fixedincome/government/treasury_auctions",
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model: treasury_auction,
        candidates: GOV_TREASURY_AUCTIONS,
        bronze_table: Some("raw.treasury_auction"),
        doc: "US Treasury security auction results (US Treasury FiscalData, keyless).",
        chartable: false,
    });
    entries.push(CatalogEntry {
        route: "fixedincome/government/treasury_prices",
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model: treasury_price,
        candidates: GOV_TREASURY_PRICES,
        bronze_table: Some("raw.treasury_price"),
        doc: "US Treasury security reference prices (US Treasury FiscalData, keyless).",
        chartable: true,
    });
    entries.push(CatalogEntry {
        route: "fixedincome/government/dealer_stats",
        kind: EndpointKind::Fetch,
        params_schema: standard_params,
        model: macro_series,
        candidates: GOV_DEALER_STATS,
        bronze_table: Some("raw.macro_series"),
        doc: "Primary dealer net positioning statistics (Federal Reserve, keyless).",
        chartable: true,
    });
    entries
}
