//! Clean-room catalog mapping `OpenBB` economy / fixedincome command paths to the
//! FRED data series that back them (gap-matrix item **L2.3**).
//!
//! Each [`FredEndpoint`] standardizes one `OpenBB` Platform command (per the
//! `docs/roadmap/openbb-surface-domains.md` economy + fixedincome tables) onto a
//! concrete FRED `series/observations` series id plus the metadata needed to
//! populate a [`tdw_domain::MacroSeries`] or [`tdw_domain::RateObservation`]
//! row. The series ids are public St. Louis Fed identifiers (facts, not source):
//! e.g. `CPIAUCSL` for headline `CPI`, `SOFR` for the Secured Overnight Financing
//! Rate, `T10Y2Y` for the 10y-2y Treasury constant-maturity spread.
//!
//! This module is intentionally dependency-free (no `http` feature, no
//! `tdw-core`/`tdw-domain`) so the catalog and its coverage tests compile and
//! run in the default offline workspace build. The HTTP fetchers in
//! `http_fetcher.rs` resolve a caller-supplied command key against this catalog
//! and reuse the existing FRED observation plumbing.

/// Which standardized [`tdw_domain`](https://docs.rs) model an endpoint maps to.
///
/// The `economy/*` series map to `MacroSeries`; the `fixedincome/*` rate, spread
/// and index series map to `RateObservation`. The discriminant lets a single
/// catalog drive both fetchers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FredModel {
    /// Standardizes to [`tdw_domain::MacroSeries`].
    Macro,
    /// Standardizes to [`tdw_domain::RateObservation`].
    Rate,
}

/// One standardized FRED-backed endpoint.
///
/// `command` is the `OpenBB` Platform command path it standardizes (e.g.
/// `"economy/cpi"`, `"fixedincome/rate/sofr"`). `series_id` is the FRED series
/// that backs it. The remaining fields carry the static normalization metadata
/// that the fetchers copy onto each emitted row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FredEndpoint {
    /// `OpenBB` command path this endpoint standardizes.
    pub command: &'static str,
    /// FRED series id backing the command (e.g. `"CPIAUCSL"`).
    pub series_id: &'static str,
    /// Which standardized domain model the fetcher emits.
    pub model: FredModel,
    /// Human-readable series title for `MacroSeries::title`.
    pub title: &'static str,
    /// Frequency label (`"monthly"`, `"quarterly"`, `"daily"`, `"annual"`).
    pub frequency: &'static str,
    /// Unit label (`"index"`, `"percent"`, `"usd"`, ...).
    pub unit: &'static str,
    /// Maturity tenor for rate/spread endpoints (e.g. `"10y"`), else `""`.
    pub maturity: &'static str,
    /// `ISO 4217` currency the value is denominated in (`"USD"`, `"EUR"`, `"GBP"`).
    pub currency: &'static str,
}

impl FredEndpoint {
    /// The maturity tenor as an [`Option`], `None` when this endpoint carries no
    /// tenor (the catalog stores `""` for "not applicable").
    #[must_use]
    pub const fn maturity_opt(&self) -> Option<&'static str> {
        if self.maturity.is_empty() {
            None
        } else {
            Some(self.maturity)
        }
    }
}

/// The standardized FRED-backed endpoint catalog.
///
/// Ordered by cluster: economy macro series, then fixedincome rate / spread /
/// index series. Every entry's `series_id` is a public FRED identifier.
pub const ENDPOINTS: &[FredEndpoint] = &[
    // -- economy: macro series (MacroSeries) ---------------------------------
    FredEndpoint {
        command: "economy/cpi",
        series_id: "CPIAUCSL",
        model: FredModel::Macro,
        title: "Consumer Price Index for All Urban Consumers: All Items",
        frequency: "monthly",
        unit: "index",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/pce",
        series_id: "PCEPI",
        model: FredModel::Macro,
        title: "Personal Consumption Expenditures: Chain-type Price Index",
        frequency: "monthly",
        unit: "index",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/gdp/real",
        series_id: "GDPC1",
        model: FredModel::Macro,
        title: "Real Gross Domestic Product",
        frequency: "quarterly",
        unit: "usd",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/gdp/nominal",
        series_id: "GDP",
        model: FredModel::Macro,
        title: "Gross Domestic Product",
        frequency: "quarterly",
        unit: "usd",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/unemployment",
        series_id: "UNRATE",
        model: FredModel::Macro,
        title: "Unemployment Rate",
        frequency: "monthly",
        unit: "percent",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/money_measures/m1",
        series_id: "M1SL",
        model: FredModel::Macro,
        title: "M1 Money Stock",
        frequency: "monthly",
        unit: "usd",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/money_measures/m2",
        series_id: "M2SL",
        model: FredModel::Macro,
        title: "M2 Money Stock",
        frequency: "monthly",
        unit: "usd",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/survey/nonfarm_payrolls",
        series_id: "PAYEMS",
        model: FredModel::Macro,
        title: "All Employees, Total Nonfarm",
        frequency: "monthly",
        unit: "thousands",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/survey/university_of_michigan",
        series_id: "UMCSENT",
        model: FredModel::Macro,
        title: "University of Michigan: Consumer Sentiment",
        frequency: "monthly",
        unit: "index",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/survey/inflation_expectations",
        series_id: "MICH",
        model: FredModel::Macro,
        title: "University of Michigan: Inflation Expectation",
        frequency: "monthly",
        unit: "percent",
        maturity: "",
        currency: "USD",
    },
    // -- economy: survey / price breadth (OpenBB-parity P4W4) ---------------
    FredEndpoint {
        command: "economy/retail_prices",
        series_id: "MRTSSM44000USS",
        model: FredModel::Macro,
        title: "Advance Retail Sales: Retail and Food Services, Total",
        frequency: "monthly",
        unit: "usd",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/survey/sloos",
        series_id: "DRTSCILM",
        model: FredModel::Macro,
        title: "Net Percentage of Banks Tightening Standards for C&I Loans",
        frequency: "quarterly",
        unit: "percent",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/survey/economic_conditions_chicago",
        series_id: "CFNAI",
        model: FredModel::Macro,
        title: "Chicago Fed National Activity Index",
        frequency: "monthly",
        unit: "index",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/survey/manufacturing_outlook_ny",
        series_id: "GACDISA066MSFRBNY",
        model: FredModel::Macro,
        title: "Empire State Manufacturing Survey: General Business Conditions",
        frequency: "monthly",
        unit: "index",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "economy/survey/manufacturing_outlook_texas",
        series_id: "BACTSAMFRBDAL",
        model: FredModel::Macro,
        title: "Texas Manufacturing Outlook Survey: General Business Activity",
        frequency: "monthly",
        unit: "index",
        maturity: "",
        currency: "USD",
    },
    // -- commodity: spot prices (MacroSeries) -------------------------------
    FredEndpoint {
        command: "commodity/price/spot",
        series_id: "DCOILWTICO",
        model: FredModel::Macro,
        title: "Crude Oil Prices: West Texas Intermediate (WTI), Cushing, Oklahoma",
        frequency: "daily",
        unit: "usd_per_barrel",
        maturity: "",
        currency: "USD",
    },
    // -- fixedincome: policy / reference rates (RateObservation) -------------
    FredEndpoint {
        command: "fixedincome/rate/sofr",
        series_id: "SOFR",
        model: FredModel::Rate,
        title: "Secured Overnight Financing Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "overnight",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/rate/effr",
        series_id: "EFFR",
        model: FredModel::Rate,
        title: "Effective Federal Funds Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "overnight",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/rate/estr",
        series_id: "ECBESTRVOLWGTTRMDMNRT",
        model: FredModel::Rate,
        title: "Euro Short-Term Rate (\u{20ac}STR)",
        frequency: "daily",
        unit: "percent",
        maturity: "overnight",
        currency: "EUR",
    },
    FredEndpoint {
        command: "fixedincome/rate/sonia",
        series_id: "IUDSOIA",
        model: FredModel::Rate,
        title: "Sterling Overnight Index Average (SONIA)",
        frequency: "daily",
        unit: "percent",
        maturity: "overnight",
        currency: "GBP",
    },
    FredEndpoint {
        command: "fixedincome/rate/ecb",
        series_id: "ECBDFR",
        model: FredModel::Rate,
        title: "ECB Deposit Facility Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "",
        currency: "EUR",
    },
    FredEndpoint {
        command: "fixedincome/rate/iorb",
        series_id: "IORB",
        model: FredModel::Rate,
        title: "Interest Rate on Reserve Balances",
        frequency: "daily",
        unit: "percent",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/rate/dpcredit",
        series_id: "DPCREDIT",
        model: FredModel::Rate,
        title: "Discount Window Primary Credit Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/rate/overnight_bank_funding",
        series_id: "OBFR",
        model: FredModel::Rate,
        title: "Overnight Bank Funding Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "overnight",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/rate/ameribor",
        series_id: "AMERIBOR",
        model: FredModel::Rate,
        title: "Overnight Unsecured AMERIBOR Benchmark Interest Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "overnight",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/rate/effr_forecast",
        series_id: "FEDTARMD",
        model: FredModel::Rate,
        title: "FOMC Summary of Economic Projections for the Fed Funds Rate, Median",
        frequency: "annual",
        unit: "percent",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/rate/effr_forecast/long_run",
        series_id: "FEDTARMDLR",
        model: FredModel::Rate,
        title: "Longer Run FOMC Projection for the Fed Funds Rate, Median",
        frequency: "annual",
        unit: "percent",
        maturity: "long_run",
        currency: "USD",
    },
    // -- fixedincome/government: Treasury constant-maturity rates ------------
    FredEndpoint {
        command: "fixedincome/government/treasury_rates/3m",
        series_id: "DGS3MO",
        model: FredModel::Rate,
        title: "Market Yield on 3-Month Treasury Constant Maturity",
        frequency: "daily",
        unit: "percent",
        maturity: "3m",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/government/treasury_rates/2y",
        series_id: "DGS2",
        model: FredModel::Rate,
        title: "Market Yield on 2-Year Treasury Constant Maturity",
        frequency: "daily",
        unit: "percent",
        maturity: "2y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/government/treasury_rates/10y",
        series_id: "DGS10",
        model: FredModel::Rate,
        title: "Market Yield on 10-Year Treasury Constant Maturity",
        frequency: "daily",
        unit: "percent",
        maturity: "10y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/government/treasury_rates/30y",
        series_id: "DGS30",
        model: FredModel::Rate,
        title: "Market Yield on 30-Year Treasury Constant Maturity",
        frequency: "daily",
        unit: "percent",
        maturity: "30y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/government/tips_yields/10y",
        series_id: "DFII10",
        model: FredModel::Rate,
        title: "Market Yield on 10-Year TIPS Constant Maturity",
        frequency: "daily",
        unit: "percent",
        maturity: "10y",
        currency: "USD",
    },
    // -- fixedincome/government: Svensson (GSW) fitted zero-coupon curve -----
    FredEndpoint {
        command: "fixedincome/government/svensson_yield_curve/2y",
        series_id: "SVENY02",
        model: FredModel::Rate,
        title: "Fitted Yield on a 2-Year Zero Coupon Bond (Svensson/GSW)",
        frequency: "daily",
        unit: "percent",
        maturity: "2y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/government/svensson_yield_curve/5y",
        series_id: "SVENY05",
        model: FredModel::Rate,
        title: "Fitted Yield on a 5-Year Zero Coupon Bond (Svensson/GSW)",
        frequency: "daily",
        unit: "percent",
        maturity: "5y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/government/svensson_yield_curve/10y",
        series_id: "SVENY10",
        model: FredModel::Rate,
        title: "Fitted Yield on a 10-Year Zero Coupon Bond (Svensson/GSW)",
        frequency: "daily",
        unit: "percent",
        maturity: "10y",
        currency: "USD",
    },
    // -- fixedincome/spreads: Treasury constant-maturity spreads ------------
    FredEndpoint {
        command: "fixedincome/spreads/tcm/10y2y",
        series_id: "T10Y2Y",
        model: FredModel::Rate,
        title: "10-Year minus 2-Year Treasury Constant Maturity",
        frequency: "daily",
        unit: "percent",
        maturity: "10y-2y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/spreads/tcm/10y3m",
        series_id: "T10Y3M",
        model: FredModel::Rate,
        title: "10-Year minus 3-Month Treasury Constant Maturity",
        frequency: "daily",
        unit: "percent",
        maturity: "10y-3m",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/spreads/treasury_effr/3m",
        series_id: "T3MFF",
        model: FredModel::Rate,
        title: "3-Month Treasury Constant Maturity minus Federal Funds Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "3m",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/spreads/tcm_effr/1y",
        series_id: "T1YFF",
        model: FredModel::Rate,
        title: "1-Year Treasury Constant Maturity minus Federal Funds Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "1y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/spreads/tcm_effr/10y",
        series_id: "T10YFF",
        model: FredModel::Rate,
        title: "10-Year Treasury Constant Maturity minus Federal Funds Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "10y",
        currency: "USD",
    },
    // -- fixedincome/corporate: HQM spot rates ------------------------------
    FredEndpoint {
        command: "fixedincome/corporate/spot_rates/10y",
        series_id: "HQMCB10YR",
        model: FredModel::Rate,
        title: "High Quality Market (HQM) Corporate Bond Spot Rate, 10-Year",
        frequency: "monthly",
        unit: "percent",
        maturity: "10y",
        currency: "USD",
    },
    // -- fixedincome/corporate: HQM corporate yield curve -------------------
    FredEndpoint {
        command: "fixedincome/corporate/hqm/2y",
        series_id: "HQMCB2YR",
        model: FredModel::Rate,
        title: "High Quality Market (HQM) Corporate Bond Spot Rate, 2-Year",
        frequency: "monthly",
        unit: "percent",
        maturity: "2y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/corporate/hqm/5y",
        series_id: "HQMCB5YR",
        model: FredModel::Rate,
        title: "High Quality Market (HQM) Corporate Bond Spot Rate, 5-Year",
        frequency: "monthly",
        unit: "percent",
        maturity: "5y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/corporate/hqm/30y",
        series_id: "HQMCB30YR",
        model: FredModel::Rate,
        title: "High Quality Market (HQM) Corporate Bond Spot Rate, 30-Year",
        frequency: "monthly",
        unit: "percent",
        maturity: "30y",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/corporate/commercial_paper/90d",
        series_id: "DCPN3M",
        model: FredModel::Rate,
        title: "90-Day AA Nonfinancial Commercial Paper Interest Rate",
        frequency: "daily",
        unit: "percent",
        maturity: "90d",
        currency: "USD",
    },
    // -- fixedincome: bond / mortgage indices -------------------------------
    FredEndpoint {
        command: "fixedincome/bond_indices/us_corporate_hy",
        series_id: "BAMLH0A0HYM2EY",
        model: FredModel::Rate,
        title: "ICE BofA US High Yield Index Effective Yield",
        frequency: "daily",
        unit: "percent",
        maturity: "",
        currency: "USD",
    },
    FredEndpoint {
        command: "fixedincome/mortgage_indices/30y_fixed",
        series_id: "MORTGAGE30US",
        model: FredModel::Rate,
        title: "30-Year Fixed Rate Mortgage Average in the United States",
        frequency: "weekly",
        unit: "percent",
        maturity: "30y",
        currency: "USD",
    },
];

/// Resolve a catalog entry by its `OpenBB` `command` path.
///
/// Returns the matching [`FredEndpoint`] or `None` when the command is not in
/// the standardized catalog (callers fall back to the generic `fred_series`
/// path, which accepts any raw series id).
#[must_use]
pub fn resolve(command: &str) -> Option<&'static FredEndpoint> {
    ENDPOINTS.iter().find(|entry| entry.command == command)
}

/// Count of standardized endpoints in each model class, `(macro_count, rate_count)`.
#[must_use]
pub fn counts() -> (usize, usize) {
    let macro_count = ENDPOINTS
        .iter()
        .filter(|e| e.model == FredModel::Macro)
        .count();
    let rate_count = ENDPOINTS
        .iter()
        .filter(|e| e.model == FredModel::Rate)
        .count();
    (macro_count, rate_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn standardizes_at_least_fifteen_endpoints() {
        // The L2.3 done-when bar is >= 15 fred-backed endpoints standardized.
        assert!(
            ENDPOINTS.len() >= 15,
            "expected >= 15 standardized endpoints, got {}",
            ENDPOINTS.len()
        );
    }

    #[test]
    fn command_paths_are_unique() {
        let mut seen = BTreeSet::new();
        for entry in ENDPOINTS {
            assert!(
                seen.insert(entry.command),
                "duplicate command path: {}",
                entry.command
            );
        }
    }

    #[test]
    fn series_ids_are_non_empty_and_uppercase() {
        for entry in ENDPOINTS {
            assert!(
                !entry.series_id.is_empty(),
                "empty series id for {}",
                entry.command
            );
            assert!(
                entry
                    .series_id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "series id {} has unexpected characters",
                entry.series_id
            );
            assert_eq!(
                entry.series_id,
                entry.series_id.to_ascii_uppercase(),
                "series id {} must be uppercase",
                entry.series_id
            );
        }
    }

    #[test]
    fn resolve_finds_known_commands_and_misses_unknown() {
        let cpi = resolve("economy/cpi").expect("cpi must resolve");
        assert_eq!(cpi.series_id, "CPIAUCSL");
        assert_eq!(cpi.model, FredModel::Macro);

        let sofr = resolve("fixedincome/rate/sofr").expect("sofr must resolve");
        assert_eq!(sofr.series_id, "SOFR");
        assert_eq!(sofr.model, FredModel::Rate);
        assert_eq!(sofr.maturity_opt(), Some("overnight"));

        assert!(resolve("equity/price/quote").is_none());
    }

    #[test]
    fn both_model_classes_are_populated() {
        let (macro_count, rate_count) = counts();
        assert!(macro_count >= 5, "macro endpoints: {macro_count}");
        assert!(rate_count >= 10, "rate endpoints: {rate_count}");
        assert_eq!(macro_count + rate_count, ENDPOINTS.len());
    }

    #[test]
    fn rate_endpoints_carry_currency_and_spreads_have_tenor() {
        for entry in ENDPOINTS.iter().filter(|e| e.model == FredModel::Rate) {
            assert!(
                entry.currency.len() == 3,
                "rate {} must carry an ISO currency, got {:?}",
                entry.command,
                entry.currency
            );
        }
        let spread = resolve("fixedincome/spreads/tcm/10y2y").expect("spread must resolve");
        assert_eq!(spread.maturity_opt(), Some("10y-2y"));
    }

    #[test]
    fn p4w5_fixedincome_fill_commands_resolve() {
        // P4W5 net-new FRED-backed fixedincome routes: each must resolve to its
        // public FRED series id and standardize onto RateObservation.
        let cases: &[(&str, &str)] = &[
            ("fixedincome/rate/ameribor", "AMERIBOR"),
            ("fixedincome/rate/effr_forecast", "FEDTARMD"),
            ("fixedincome/rate/effr_forecast/long_run", "FEDTARMDLR"),
            ("fixedincome/government/svensson_yield_curve/2y", "SVENY02"),
            ("fixedincome/government/svensson_yield_curve/5y", "SVENY05"),
            ("fixedincome/government/svensson_yield_curve/10y", "SVENY10"),
            ("fixedincome/spreads/tcm_effr/1y", "T1YFF"),
            ("fixedincome/spreads/tcm_effr/10y", "T10YFF"),
            ("fixedincome/corporate/hqm/2y", "HQMCB2YR"),
            ("fixedincome/corporate/hqm/5y", "HQMCB5YR"),
            ("fixedincome/corporate/hqm/30y", "HQMCB30YR"),
        ];
        for (command, series_id) in cases {
            let entry =
                resolve(command).unwrap_or_else(|| panic!("{command} must resolve in catalog"));
            assert_eq!(entry.series_id, *series_id, "series id for {command}");
            assert_eq!(entry.model, FredModel::Rate, "model for {command}");
        }
    }

    #[test]
    fn macro_endpoints_have_no_tenor() {
        for entry in ENDPOINTS.iter().filter(|e| e.model == FredModel::Macro) {
            assert_eq!(
                entry.maturity_opt(),
                None,
                "macro {} should not carry a maturity",
                entry.command
            );
        }
    }
}
