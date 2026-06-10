//! Standard data models for the big OpenBB-surface clusters (gap-matrix item **L1.4**).
//!
//! Field names follow the cluster tables in `docs/roadmap/openbb-surface-domains.md`
//! (clean-room: derived from public surface docs, not upstream source code). Provider
//! coverage for these clusters is intentionally wide and inconsistent, so most fields
//! are [`Option`] — a model is the *union* of what providers can supply, and each
//! provider fills the subset it serves. The handful of identity/anchor fields that
//! every provider must supply use the crate's `#[validate(length(min = 1))]` idiom.
//!
//! Statement-style models that carry many provider-variable numeric line items use a
//! `BTreeMap<String, f64>` `line_items` bag (stable-ordered for deterministic
//! snapshots) rather than enumerating every possible line, while keeping the common
//! headers as typed fields.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::Validate;

/// Which financial statement a [`FinancialStatement`] represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatementKind {
    /// Balance sheet (`equity/fundamental/balance`).
    Balance,
    /// Income statement (`equity/fundamental/income`).
    Income,
    /// Cash-flow statement (`equity/fundamental/cash`).
    Cash,
}

/// Standardized financial statement covering the balance / income / cash variants.
///
/// Standardizes: `equity/fundamental/balance`, `equity/fundamental/income`,
/// `equity/fundamental/cash` (and, via [`StatementKind`], their growth-rate siblings
/// `*_growth`). The common period header is typed; the many provider-variable
/// statement lines live in `line_items` keyed by a normalized `snake_case` name
/// (e.g. `total_assets`, `net_income`, `operating_cash_flow`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FinancialStatement {
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Which statement this row represents.
    pub statement: StatementKind,
    /// Reporting period label, e.g. `"annual"` or `"quarter"`.
    #[validate(length(min = 1))]
    pub period: String,
    /// Fiscal year the statement covers.
    pub fiscal_year: Option<i32>,
    /// Fiscal period label, e.g. `"FY"`, `"Q1"`.
    pub fiscal_period: Option<String>,
    /// Period end date (statement date).
    pub date: Option<String>,
    /// Date the statement was filed/accepted.
    pub filing_date: Option<String>,
    /// Reporting currency (ISO 4217), provider-variable.
    pub currency: Option<String>,
    /// Standardized statement line items keyed by normalized name.
    #[serde(default)]
    pub line_items: BTreeMap<String, f64>,
}

/// Key financial metrics (per-share and valuation).
///
/// Standardizes: `equity/fundamental/metrics`. Per the docs this is a wide,
/// provider-variable cluster (finviz/fmp/intrinio/yfinance), so all metrics are
/// optional.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct KeyMetrics {
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Reporting period the metrics describe.
    pub period: Option<String>,
    /// Period end date.
    pub date: Option<String>,
    pub market_cap: Option<f64>,
    pub pe_ratio: Option<f64>,
    pub price_to_sales: Option<f64>,
    pub price_to_book: Option<f64>,
    pub enterprise_value: Option<f64>,
    pub ev_to_ebitda: Option<f64>,
    pub earnings_per_share: Option<f64>,
    pub revenue_per_share: Option<f64>,
    pub book_value_per_share: Option<f64>,
    pub free_cash_flow_per_share: Option<f64>,
    pub dividend_yield: Option<f64>,
    /// Additional provider-specific metrics keyed by normalized name.
    #[serde(default)]
    pub extra_metrics: BTreeMap<String, f64>,
}

/// Financial ratios (liquidity, profitability, leverage, efficiency).
///
/// Standardizes: `equity/fundamental/ratios`. All ratios optional (fmp/intrinio
/// supply differing subsets).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Ratios {
    #[validate(length(min = 1))]
    pub symbol: String,
    pub period: Option<String>,
    pub date: Option<String>,
    pub current_ratio: Option<f64>,
    pub quick_ratio: Option<f64>,
    pub gross_margin: Option<f64>,
    pub operating_margin: Option<f64>,
    pub net_profit_margin: Option<f64>,
    pub return_on_assets: Option<f64>,
    pub return_on_equity: Option<f64>,
    pub debt_to_equity: Option<f64>,
    pub interest_coverage: Option<f64>,
    pub asset_turnover: Option<f64>,
    /// Additional provider-specific ratios keyed by normalized name.
    #[serde(default)]
    pub extra_ratios: BTreeMap<String, f64>,
}

/// Analyst estimate (price target, consensus, or forward estimate).
///
/// Standardizes the `equity/estimates/*` cluster: `price_target`, `consensus`,
/// `forward_eps`, `forward_sales`, `forward_ebitda`, `forward_pe`, and `historical`.
/// The `kind` field discriminates which estimate this row carries; numeric fields
/// are optional so one shape serves every variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Estimate {
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Estimate variant, e.g. `"price_target"`, `"consensus"`, `"forward_eps"`.
    #[validate(length(min = 1))]
    pub kind: String,
    /// Fiscal period the estimate targets, e.g. `"2026"`, `"2026Q2"`.
    pub fiscal_period: Option<String>,
    /// Date the estimate was published/observed.
    pub date: Option<String>,
    /// Analyst or firm name (for price targets / analyst forecasts).
    pub analyst: Option<String>,
    /// Recommendation label, e.g. `"buy"`, `"hold"`, `"sell"`.
    pub recommendation: Option<String>,
    /// Headline estimate value (target price, forward EPS/sales/EBITDA, etc.).
    pub value: Option<f64>,
    /// Low end of the estimate range.
    pub low: Option<f64>,
    /// High end of the estimate range.
    pub high: Option<f64>,
    /// Mean / consensus estimate.
    pub mean: Option<f64>,
    /// Number of contributing analysts.
    pub number_of_analysts: Option<u32>,
    pub currency: Option<String>,
}

/// A single observation in a macroeconomic time series.
///
/// Standardizes the `economy/*` series cluster — `cpi`, `pce`, `gdp/*`,
/// `unemployment`, `interest_rates`, the `survey/*` series, and the arbitrary
/// `economy/fred_series` / `fred_regional` lookups. One row = one (series, date)
/// observation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct MacroSeries {
    /// Series identifier (e.g. a FRED series id like `"CPIAUCSL"`).
    #[validate(length(min = 1))]
    pub series_id: String,
    /// Human-readable series name/title.
    pub title: Option<String>,
    /// Observation date.
    #[validate(length(min = 1))]
    pub date: String,
    /// Observed value (optional: some series report missing observations).
    pub value: Option<f64>,
    /// ISO 3166-1 alpha-2 country code where applicable.
    pub country: Option<String>,
    /// Frequency label, e.g. `"monthly"`, `"quarterly"`, `"annual"`.
    pub frequency: Option<String>,
    /// Unit of measure, e.g. `"index"`, `"percent"`, `"usd"`.
    pub unit: Option<String>,
    /// Transform applied, e.g. `"yoy"`, `"pop"`, `"level"`.
    pub transform: Option<String>,
}

/// A single interest-rate / spread observation.
///
/// Standardizes the `fixedincome/rate/*` and `fixedincome/spreads/*` clusters plus
/// `government/treasury_rates`, `bond_indices`, and `mortgage_indices`. One row =
/// one (rate, date) observation, optionally tagged with a maturity tenor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct RateObservation {
    /// Rate identifier, e.g. `"sofr"`, `"effr"`, `"tcm_10y"`.
    #[validate(length(min = 1))]
    pub rate_id: String,
    /// Observation date.
    #[validate(length(min = 1))]
    pub date: String,
    /// Observed rate (typically percent). Optional for missing observations.
    pub value: Option<f64>,
    /// Maturity tenor label where applicable, e.g. `"3m"`, `"10y"`.
    pub maturity: Option<String>,
    /// ISO 4217 currency the rate is denominated in.
    pub currency: Option<String>,
}

/// A single point on a sovereign yield curve.
///
/// Standardizes `fixedincome/government/yield_curve`: one row = one (date,
/// maturity) constant-maturity Treasury yield. The aggregating fetcher emits one
/// `YieldCurvePoint` per (observation date, tenor) by merging the individual
/// constant-maturity series (3m / 2y / 10y / 30y). `value` is optional for
/// missing observations, mirroring [`RateObservation`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct YieldCurvePoint {
    /// Curve identifier, e.g. `"us_treasury"`.
    #[validate(length(min = 1))]
    pub curve_id: String,
    /// Observation date.
    #[validate(length(min = 1))]
    pub date: String,
    /// Maturity tenor label, e.g. `"3m"`, `"2y"`, `"10y"`, `"30y"`.
    #[validate(length(min = 1))]
    pub maturity: String,
    /// Observed yield (percent). Optional for missing observations.
    pub value: Option<f64>,
    /// ISO 4217 currency the yield is denominated in.
    pub currency: Option<String>,
}

/// A FRED series-search metadata result.
///
/// Standardizes `economy/fred_search` (FRED `series/search`). One row = one
/// matched series; all descriptive fields beyond the id are optional since the
/// search API may omit them. Carries no observations — it is a discovery result
/// that callers feed into the macro/rate observation endpoints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SeriesSearchResult {
    /// FRED series id, e.g. `"CPIAUCSL"`.
    #[validate(length(min = 1))]
    pub series_id: String,
    /// Human-readable series title.
    pub title: Option<String>,
    /// Frequency label, e.g. `"Monthly"`, `"Daily"`.
    pub frequency: Option<String>,
    /// Units label, e.g. `"Index 1982-1984=100"`.
    pub units: Option<String>,
    /// FRED popularity score (higher is more popular).
    pub popularity: Option<i64>,
}

/// A single option contract within a chain.
///
/// Standardizes `derivatives/options/chains` (cboe/deribit/intrinio/tmx/tradier/
/// yfinance). Greeks and quote fields are optional since provider coverage varies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct OptionContract {
    /// Underlying symbol.
    #[validate(length(min = 1))]
    pub underlying_symbol: String,
    /// Contract symbol / OCC-style identifier where available.
    pub contract_symbol: Option<String>,
    /// Expiration date.
    #[validate(length(min = 1))]
    pub expiration: String,
    /// Strike price.
    pub strike: f64,
    /// `"call"` or `"put"`.
    #[validate(length(min = 1))]
    pub option_type: String,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub last_price: Option<f64>,
    pub volume: Option<u64>,
    pub open_interest: Option<u64>,
    pub implied_volatility: Option<f64>,
    pub delta: Option<f64>,
    pub gamma: Option<f64>,
    pub theta: Option<f64>,
    pub vega: Option<f64>,
    pub rho: Option<f64>,
}

/// A news article (company-specific or world).
///
/// Standardizes `news/company` and `news/world` (benzinga/biztoc/fmp/intrinio/
/// polygon/tiingo/tmx/yfinance).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct NewsArticle {
    /// Stable article identifier (provider id or URL-derived).
    #[validate(length(min = 1))]
    pub id: String,
    /// Article title/headline.
    #[validate(length(min = 1))]
    pub title: String,
    /// Publication timestamp (RFC 3339).
    #[validate(length(min = 1))]
    pub published_at: String,
    /// Article body / summary text.
    pub text: Option<String>,
    /// Source URL.
    pub url: Option<String>,
    /// Publisher / source name.
    pub source: Option<String>,
    /// Author byline.
    pub author: Option<String>,
    /// Symbols the article is tagged with.
    #[serde(default)]
    pub symbols: Vec<String>,
}

/// An ownership / insider / institutional-holding record.
///
/// Standardizes the `equity/ownership/*` cluster: `insider_trading`,
/// `institutional`, `major_holders`, `share_statistics`, `form_13f`, and
/// `government_trades`. The `kind` field discriminates the record type; most fields
/// are optional since each ownership endpoint reports a different subset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct OwnershipRecord {
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Record variant, e.g. `"insider"`, `"institutional"`, `"form_13f"`,
    /// `"share_statistics"`, `"government_trade"`.
    #[validate(length(min = 1))]
    pub kind: String,
    /// Holder / filer / insider name.
    pub holder: Option<String>,
    /// Relationship or role (e.g. insider title, chamber for gov trades).
    pub relationship: Option<String>,
    /// Date the holding/transaction was reported or filed.
    pub date: Option<String>,
    /// Transaction type where applicable, e.g. `"buy"`, `"sell"`.
    pub transaction_type: Option<String>,
    /// Number of shares held or transacted.
    pub shares: Option<f64>,
    /// Market value of the holding/transaction.
    pub value: Option<f64>,
    /// Percentage of outstanding shares (for ownership-percentage records).
    pub percentage: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + core::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize model");
        let back: T = serde_json::from_str(&json).expect("deserialize model");
        assert_eq!(value, &back);
    }

    #[test]
    fn financial_statement_round_trips_and_validates() {
        let mut line_items = BTreeMap::new();
        line_items.insert("total_assets".to_string(), 352_755_000_000.0);
        line_items.insert("total_liabilities".to_string(), 290_437_000_000.0);
        let stmt = FinancialStatement {
            symbol: "AAPL".to_string(),
            statement: StatementKind::Balance,
            period: "annual".to_string(),
            fiscal_year: Some(2025),
            fiscal_period: Some("FY".to_string()),
            date: Some("2025-09-27".to_string()),
            filing_date: Some("2025-11-01".to_string()),
            currency: Some("USD".to_string()),
            line_items,
        };
        assert!(stmt.validate().is_ok());
        round_trip(&stmt);

        let bad = FinancialStatement {
            symbol: String::new(),
            ..stmt
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn key_metrics_round_trips_and_validates() {
        let metrics = KeyMetrics {
            symbol: "AAPL".to_string(),
            period: Some("annual".to_string()),
            date: Some("2025-09-27".to_string()),
            market_cap: Some(3.5e12),
            pe_ratio: Some(31.2),
            price_to_sales: Some(8.1),
            price_to_book: Some(48.0),
            enterprise_value: Some(3.6e12),
            ev_to_ebitda: Some(24.0),
            earnings_per_share: Some(6.5),
            revenue_per_share: Some(25.0),
            book_value_per_share: Some(4.2),
            free_cash_flow_per_share: Some(6.0),
            dividend_yield: Some(0.005),
            extra_metrics: BTreeMap::new(),
        };
        assert!(metrics.validate().is_ok());
        round_trip(&metrics);
    }

    #[test]
    fn ratios_round_trips_and_validates() {
        let ratios = Ratios {
            symbol: "AAPL".to_string(),
            period: Some("annual".to_string()),
            date: Some("2025-09-27".to_string()),
            current_ratio: Some(0.95),
            quick_ratio: Some(0.9),
            gross_margin: Some(0.46),
            operating_margin: Some(0.30),
            net_profit_margin: Some(0.25),
            return_on_assets: Some(0.28),
            return_on_equity: Some(1.5),
            debt_to_equity: Some(1.8),
            interest_coverage: Some(40.0),
            asset_turnover: Some(1.1),
            extra_ratios: BTreeMap::new(),
        };
        assert!(ratios.validate().is_ok());
        round_trip(&ratios);
    }

    #[test]
    fn estimate_round_trips_and_validates() {
        let est = Estimate {
            symbol: "AAPL".to_string(),
            kind: "price_target".to_string(),
            fiscal_period: Some("2026".to_string()),
            date: Some("2026-01-15".to_string()),
            analyst: Some("Jane Doe".to_string()),
            recommendation: Some("buy".to_string()),
            value: Some(250.0),
            low: Some(220.0),
            high: Some(280.0),
            mean: Some(250.0),
            number_of_analysts: Some(34),
            currency: Some("USD".to_string()),
        };
        assert!(est.validate().is_ok());
        round_trip(&est);

        let bad = Estimate {
            kind: String::new(),
            ..est
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn macro_series_round_trips_and_validates() {
        let obs = MacroSeries {
            series_id: "CPIAUCSL".to_string(),
            title: Some("Consumer Price Index for All Urban Consumers".to_string()),
            date: "2026-04-01".to_string(),
            value: Some(312.4),
            country: Some("US".to_string()),
            frequency: Some("monthly".to_string()),
            unit: Some("index".to_string()),
            transform: Some("level".to_string()),
        };
        assert!(obs.validate().is_ok());
        round_trip(&obs);

        let missing = MacroSeries {
            value: None,
            ..obs.clone()
        };
        assert!(missing.validate().is_ok());
        round_trip(&missing);

        let bad = MacroSeries {
            date: String::new(),
            ..obs
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn rate_observation_round_trips_and_validates() {
        let rate = RateObservation {
            rate_id: "sofr".to_string(),
            date: "2026-06-06".to_string(),
            value: Some(5.31),
            maturity: None,
            currency: Some("USD".to_string()),
        };
        assert!(rate.validate().is_ok());
        round_trip(&rate);

        let bad = RateObservation {
            rate_id: String::new(),
            ..rate
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn yield_curve_point_round_trips_and_validates() {
        let point = YieldCurvePoint {
            curve_id: "us_treasury".to_string(),
            date: "2026-06-06".to_string(),
            maturity: "10y".to_string(),
            value: Some(4.21),
            currency: Some("USD".to_string()),
        };
        assert!(point.validate().is_ok());
        round_trip(&point);

        let missing = YieldCurvePoint {
            value: None,
            ..point.clone()
        };
        assert!(missing.validate().is_ok());
        round_trip(&missing);

        let bad = YieldCurvePoint {
            maturity: String::new(),
            ..point
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn series_search_result_round_trips_and_validates() {
        let result = SeriesSearchResult {
            series_id: "CPIAUCSL".to_string(),
            title: Some("Consumer Price Index for All Urban Consumers".to_string()),
            frequency: Some("Monthly".to_string()),
            units: Some("Index 1982-1984=100".to_string()),
            popularity: Some(95),
        };
        assert!(result.validate().is_ok());
        round_trip(&result);

        let bad = SeriesSearchResult {
            series_id: String::new(),
            ..result
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn option_contract_round_trips_and_validates() {
        let contract = OptionContract {
            underlying_symbol: "AAPL".to_string(),
            contract_symbol: Some("AAPL260116C00250000".to_string()),
            expiration: "2026-01-16".to_string(),
            strike: 250.0,
            option_type: "call".to_string(),
            bid: Some(12.3),
            ask: Some(12.5),
            last_price: Some(12.4),
            volume: Some(1500),
            open_interest: Some(20_000),
            implied_volatility: Some(0.28),
            delta: Some(0.55),
            gamma: Some(0.02),
            theta: Some(-0.05),
            vega: Some(0.30),
            rho: Some(0.10),
        };
        assert!(contract.validate().is_ok());
        round_trip(&contract);

        let bad = OptionContract {
            option_type: String::new(),
            ..contract
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn news_article_round_trips_and_validates() {
        let article = NewsArticle {
            id: "bz-12345".to_string(),
            title: "Apple unveils new product line".to_string(),
            published_at: "2026-06-06T14:30:00Z".to_string(),
            text: Some("Full article body.".to_string()),
            url: Some("https://example.com/article".to_string()),
            source: Some("benzinga".to_string()),
            author: Some("Reporter".to_string()),
            symbols: vec!["AAPL".to_string()],
        };
        assert!(article.validate().is_ok());
        round_trip(&article);

        let bad = NewsArticle {
            title: String::new(),
            ..article
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn ownership_record_round_trips_and_validates() {
        let record = OwnershipRecord {
            symbol: "AAPL".to_string(),
            kind: "insider".to_string(),
            holder: Some("Tim Cook".to_string()),
            relationship: Some("CEO".to_string()),
            date: Some("2026-05-01".to_string()),
            transaction_type: Some("sell".to_string()),
            shares: Some(50_000.0),
            value: Some(12_500_000.0),
            percentage: None,
        };
        assert!(record.validate().is_ok());
        round_trip(&record);

        let bad = OwnershipRecord {
            kind: String::new(),
            ..record
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn statement_kind_serializes_snake_case() {
        let json = serde_json::to_string(&StatementKind::Cash).expect("serialize kind");
        assert_eq!(json, "\"cash\"");
    }
}
