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

/// A single row from an energy / commodity statistical report.
///
/// Standardizes the EIA report cluster — `commodity/petroleum_status_report`
/// (the Weekly Petroleum Status Report) and `commodity/short_term_energy_outlook`
/// (STEO). One row = one (report, series, period) observation. EIA v2 returns a
/// `period` (the observation date/label), a series identifier and human-readable
/// description, the numeric value, and a units label; coverage of the optional
/// descriptive fields varies by series, so they are [`Option`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CommodityReportRow {
    /// Report identifier the row belongs to, e.g. `"petroleum_status_report"`,
    /// `"short_term_energy_outlook"`.
    #[validate(length(min = 1))]
    pub report: String,
    /// Series identifier within the report (EIA `series` / route id).
    #[validate(length(min = 1))]
    pub series_id: String,
    /// Observation period (`YYYY-MM-DD`, `YYYY-MM`, or `YYYY` per frequency).
    #[validate(length(min = 1))]
    pub period: String,
    /// Human-readable series description.
    pub series_description: Option<String>,
    /// Numeric value (optional: some periods report no value).
    pub value: Option<f64>,
    /// Unit of measure, e.g. `"thousand barrels"`, `"dollars per barrel"`.
    pub units: Option<String>,
}

/// A single CFTC Commitments of Traders (COT) report row.
///
/// Standardizes `regulators/cftc/cot` (CFTC legacy futures-only COT report). One
/// row = one (market, report date) observation. The CFTC Socrata dataset reports
/// open interest plus the long/short breakdown across the trader categories
/// (noncommercial, commercial, total reportable, nonreportable). Every position
/// count is [`Option`] since the dataset occasionally omits a column for a given
/// market/week; only the market identity and report date are required anchors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CommitmentOfTraders {
    /// Market and exchange name, e.g.
    /// `"WHEAT-SRW - CHICAGO BOARD OF TRADE"`.
    #[validate(length(min = 1))]
    pub market: String,
    /// Report date in `YYYY-MM-DD` form (the as-of Tuesday of the report week).
    #[validate(length(min = 1))]
    pub report_date: String,
    /// Total open interest across all traders.
    pub open_interest: Option<f64>,
    /// Noncommercial (speculative) long positions.
    pub noncommercial_long: Option<f64>,
    /// Noncommercial (speculative) short positions.
    pub noncommercial_short: Option<f64>,
    /// Commercial (hedger) long positions.
    pub commercial_long: Option<f64>,
    /// Commercial (hedger) short positions.
    pub commercial_short: Option<f64>,
    /// Total reportable long positions.
    pub total_reportable_long: Option<f64>,
    /// Total reportable short positions.
    pub total_reportable_short: Option<f64>,
    /// Nonreportable long positions.
    pub nonreportable_long: Option<f64>,
    /// Nonreportable short positions.
    pub nonreportable_short: Option<f64>,
}

/// A single US Congress legislative bill record.
///
/// Standardizes the `uscongress/{bills,bill_info}` routes (congress.gov public
/// API). One row = one bill. The list endpoint (`uscongress/bills`) fills the
/// header fields; the detail endpoint (`uscongress/bill_info`) additionally
/// fills `sponsor` and `policy_area`. Every field beyond the bill identity
/// (`congress`/`bill_type`/`bill_number`) is [`Option`] since the list and
/// detail responses report different subsets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CongressBill {
    /// Congress number, e.g. `118`.
    pub congress: u32,
    /// Bill type, e.g. `"hr"`, `"s"`, `"hjres"` (lowercased).
    #[validate(length(min = 1))]
    pub bill_type: String,
    /// Bill number within the (congress, type), e.g. `"3076"`.
    #[validate(length(min = 1))]
    pub bill_number: String,
    /// Bill title.
    pub title: Option<String>,
    /// Originating chamber, e.g. `"House"` or `"Senate"`.
    pub origin_chamber: Option<String>,
    /// Date the bill was introduced (`YYYY-MM-DD`).
    pub introduced_date: Option<String>,
    /// Text of the latest action recorded on the bill.
    pub latest_action: Option<String>,
    /// Date of the latest action (`YYYY-MM-DD`).
    pub latest_action_date: Option<String>,
    /// Last-updated date reported by the API (`YYYY-MM-DD`).
    pub update_date: Option<String>,
    /// Lead sponsor's full name (detail endpoint only).
    pub sponsor: Option<String>,
    /// Policy-area label (detail endpoint only).
    pub policy_area: Option<String>,
    /// congress.gov API URL for the bill resource.
    pub url: Option<String>,
}

/// A single downloadable text version of a US Congress bill.
///
/// Standardizes the `uscongress/bill_text_urls` route (congress.gov public API
/// bill `/text` endpoint). One row = one (text version, format) pair: a bill
/// text version (e.g. "Introduced in House") is published in several formats
/// (Formatted Text, PDF, XML), each with its own URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct BillTextUrl {
    /// Text-version label, e.g. `"Introduced in House"`.
    pub version_type: Option<String>,
    /// Date the text version was published (`YYYY-MM-DD`).
    pub date: Option<String>,
    /// Format label, e.g. `"Formatted Text"`, `"PDF"`, `"XML"`.
    #[validate(length(min = 1))]
    pub format_type: String,
    /// Download URL for this text version in this format.
    #[validate(length(min = 1))]
    pub url: String,
}

/// A single research-factor return observation.
///
/// Standardizes the Ken French Data Library research factors
/// (`economy/factors/famafrench`): one row = one (date) observation of the
/// factor set the requested dataset provides. The 3-factor dataset populates
/// `mkt_rf`/`smb`/`hml`/`rf`; the 5-factor dataset adds `rmw`/`cma`; the
/// momentum dataset populates only `mom`. Every factor is [`Option`] since each
/// dataset reports a different subset, and values are decimal returns (the
/// source publishes percent — the fetcher converts percent → fraction). The
/// `date` is the bare calendar date the row covers (`YYYY-MM-DD` for daily,
/// `YYYY-MM` for monthly).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FactorReturn {
    /// Observation date: `YYYY-MM-DD` (daily) or `YYYY-MM` (monthly).
    #[validate(length(min = 1))]
    pub date: String,
    /// Excess return on the market (`Mkt-RF`), as a decimal fraction.
    pub mkt_rf: Option<f64>,
    /// Small-minus-big size factor (`SMB`), as a decimal fraction.
    pub smb: Option<f64>,
    /// High-minus-low value factor (`HML`), as a decimal fraction.
    pub hml: Option<f64>,
    /// Robust-minus-weak profitability factor (`RMW`, 5-factor only).
    pub rmw: Option<f64>,
    /// Conservative-minus-aggressive investment factor (`CMA`, 5-factor only).
    pub cma: Option<f64>,
    /// Momentum factor (`Mom`, momentum dataset only), as a decimal fraction.
    pub mom: Option<f64>,
    /// Risk-free rate (`RF`), as a decimal fraction.
    pub rf: Option<f64>,
}

/// A single portfolio-return observation from the Ken French Data Library.
///
/// Standardizes the portfolio-formation return routes
/// (`economy/factors/famafrench/{us,regional,country}_portfolio_returns` and
/// `economy/factors/famafrench/international_index_returns`). The Data Library
/// publishes these as wide tables — a leading date column followed by one column
/// per formed portfolio (e.g. `SMALL LoBM`, `BIG HiBM`) or per region/index. To
/// keep a stable, warehouse-friendly grain, each wide cell becomes one long row:
/// one (date, portfolio) observation. `value` is a decimal return (the source
/// publishes percent — the fetcher converts percent → fraction); it is
/// [`Option`] so the library's missing-value sentinels map to absent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct PortfolioReturn {
    /// Observation date: `YYYY-MM-DD` (daily) or `YYYY-MM` (monthly).
    #[validate(length(min = 1))]
    pub date: String,
    /// Formed-portfolio (or region / index) column label, e.g. `"SMALL LoBM"`.
    #[validate(length(min = 1))]
    pub portfolio: String,
    /// Portfolio return for the (date, portfolio) cell, as a decimal fraction.
    pub value: Option<f64>,
}

/// A single portfolio-formation breakpoint observation from the Ken French Data
/// Library.
///
/// Standardizes `economy/factors/famafrench/breakpoints`. The breakpoint files
/// are wide tables: a leading date column, an optional count column, then the
/// percentile breakpoint columns (e.g. the size or book-to-market deciles). Each
/// wide cell becomes one long row: one (date, breakpoint) observation. `value`
/// is [`Option`] so blank / sentinel cells map to absent. Breakpoint values are
/// the source's native units (the library publishes them as levels, not
/// percent), so the fetcher carries them through unscaled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Breakpoint {
    /// Observation date: `YYYY-MM-DD` (daily) or `YYYY-MM` (monthly).
    #[validate(length(min = 1))]
    pub date: String,
    /// Breakpoint column label, e.g. `"5"`, `"95"`, or `"Count"`.
    #[validate(length(min = 1))]
    pub breakpoint: String,
    /// Breakpoint value for the (date, breakpoint) cell, in source units.
    pub value: Option<f64>,
}

/// A single IMF SDMX discovery record (dataflow, table, dimension, or
/// presentation-table cell).
///
/// Standardizes the `imf_utils/*` SDMX discovery helpers
/// (`list_dataflows`, `list_tables`, `get_dataflow_dimensions`,
/// `presentation_table`). The IMF Data Services SDMX-JSON service exposes
/// `Dataflow` and `DataStructure` discovery shapes that are distinct from the
/// `CompactData` observation envelope; one record shape serves all four helpers
/// because they share a discovery grain (an identifier, a human-readable name,
/// and a handful of variant-specific descriptors). The `kind` field
/// discriminates the variant; most fields are [`Option`] since each helper
/// reports a different subset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ImfDiscoveryRecord {
    /// Record variant: `"dataflow"`, `"table"`, `"dimension"`, or
    /// `"presentation_cell"`.
    #[validate(length(min = 1))]
    pub kind: String,
    /// Primary identifier for the record (dataflow id, table id, dimension id,
    /// or presentation-table cell key).
    #[validate(length(min = 1))]
    pub id: String,
    /// Human-readable name / description, where the service reports one.
    pub name: Option<String>,
    /// SDMX dataflow id this record belongs to (set for table / dimension /
    /// presentation rows; the dataflow's own id for `"dataflow"` records).
    pub dataflow: Option<String>,
    /// SDMX `DataStructure` (DSD) id backing the dataflow, when reported.
    pub structure: Option<String>,
    /// Dimension position within the SDMX key order (dimension records only).
    pub position: Option<u32>,
    /// Presentation-table cell value (presentation rows only).
    pub value: Option<String>,
}

/// A scheduled corporate-calendar event (dividend, earnings, or IPO).
///
/// Standardizes the NASDAQ calendar cluster — `equity/calendar/dividends`,
/// `equity/calendar/earnings`, and `equity/calendar/ipo`. The `kind` field
/// discriminates the event variant; one shape serves all three because the
/// public NASDAQ calendar feeds share a row grain (a symbol, an event date, and
/// a handful of variant-specific numeric/textual fields). Most fields are
/// [`Option`] since each calendar reports a different subset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CalendarEvent {
    /// Event variant: `"dividend"`, `"earnings"`, or `"ipo"`.
    #[validate(length(min = 1))]
    pub kind: String,
    /// Ticker symbol the event concerns.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Company / security name where the feed reports it.
    pub name: Option<String>,
    /// Primary event date (ex-dividend date, earnings date, or expected IPO
    /// date), in `YYYY-MM-DD` form.
    pub date: Option<String>,
    /// Dividend amount per share (dividends only).
    pub dividend: Option<f64>,
    /// Payment date (dividends only), in `YYYY-MM-DD` form.
    pub payment_date: Option<String>,
    /// Record date (dividends only), in `YYYY-MM-DD` form.
    pub record_date: Option<String>,
    /// Consensus / reported EPS estimate (earnings only).
    pub eps_estimate: Option<f64>,
    /// Fiscal period the earnings row covers, e.g. `"2026Q1"` (earnings only).
    pub fiscal_period: Option<String>,
    /// IPO offer price (IPO only).
    pub price: Option<f64>,
    /// Number of shares offered (IPO only).
    pub shares: Option<f64>,
    /// Exchange the security lists on (IPO only).
    pub exchange: Option<String>,
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

/// A ticker-symbol ↔ CIK mapping row.
///
/// Standardizes `regulators/sec/cik_map` / `regulators/sec/symbol_map` (SEC
/// `company_tickers.json`). One row = one (ticker, CIK) pair, optionally with
/// the issuer's reported company name. The CIK is carried as a string so leading
/// zeros and the canonical zero-padded form survive a round trip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SymbolMapping {
    /// Exchange ticker symbol (upper-cased by the provider fetcher).
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Central Index Key (CIK) as reported by SEC, unpadded.
    #[validate(length(min = 1))]
    pub cik: String,
    /// Issuer / company name where the source supplies it.
    pub name: Option<String>,
}

/// A single ETF / fund portfolio holding.
///
/// Standardizes `etf/holdings` (SEC N-PORT `NPORT-P` portfolio disclosure). One
/// row = one constituent the fund reports holding on a given report date. Most
/// fields are optional because N-PORT filers populate a variable subset
/// (identifiers, valuation, weight). Identity anchors are the fund handle and
/// the holding's name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EtfHolding {
    /// Fund handle: the ETF ticker when known, else the filer CIK.
    #[validate(length(min = 1))]
    pub fund_symbol: String,
    /// Filer Central Index Key (CIK), unpadded.
    pub cik: Option<String>,
    /// Portfolio report (period-of-report) date.
    pub report_date: Option<String>,
    /// Holding / security name as reported.
    #[validate(length(min = 1))]
    pub holding_name: String,
    /// CUSIP identifier where reported.
    pub cusip: Option<String>,
    /// ISIN identifier where reported.
    pub isin: Option<String>,
    /// Balance (units / shares / par) held.
    pub balance: Option<f64>,
    /// Market value of the holding in USD.
    pub value_usd: Option<f64>,
    /// Percentage weight of the holding in the portfolio.
    pub weight_pct: Option<f64>,
}

/// A single sector / asset-class weight for an ETF or fund.
///
/// Standardizes `etf/sectors` (derived from SEC N-PORT). One row = one
/// (fund, sector) allocation. `weight_pct` is optional because some N-PORT
/// filings omit a usable per-sector breakdown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EtfSectorWeight {
    /// Fund handle: the ETF ticker when known, else the filer CIK.
    #[validate(length(min = 1))]
    pub fund_symbol: String,
    /// Portfolio report (period-of-report) date.
    pub report_date: Option<String>,
    /// Sector / asset-class label.
    #[validate(length(min = 1))]
    pub sector: String,
    /// Percentage weight of the sector in the portfolio.
    pub weight_pct: Option<f64>,
}

/// ETF profile / reference information.
///
/// Standardizes `etf/info` (FMP `etf/info`, Yahoo `quoteSummary`). One row = one
/// fund. `symbol` and `name` are the identity anchors; the descriptive and
/// numeric attributes are [`Option`] because provider coverage of each field
/// varies by fund and venue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EtfInfo {
    /// Fund ticker symbol.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Fund name as reported.
    #[validate(length(min = 1))]
    pub name: String,
    /// Issuer / fund family, e.g. `"Vanguard"`.
    pub issuer: Option<String>,
    /// Net asset value per share.
    pub nav: Option<f64>,
    /// Total net assets under management (listing currency).
    pub aum: Option<f64>,
    /// Annual expense ratio (fraction, e.g. `0.0003` is `0.03%`).
    pub expense_ratio: Option<f64>,
    /// Number of constituent holdings.
    pub holdings_count: Option<u64>,
    /// Listing currency, e.g. `"USD"`.
    pub currency: Option<String>,
    /// Listing exchange, e.g. `"NYSE Arca"`.
    pub exchange: Option<String>,
    /// Inception date (`YYYY-MM-DD`).
    pub inception_date: Option<String>,
    /// Free-text investment description / objective.
    pub description: Option<String>,
}

/// A single country / domicile weight for an ETF or fund.
///
/// Standardizes `etf/countries` (FMP). One row = one (fund, country) allocation.
/// `weight_pct` is optional because some funds omit a usable per-country
/// breakdown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EtfCountryWeight {
    /// Fund ticker symbol.
    #[validate(length(min = 1))]
    pub fund_symbol: String,
    /// Country label as reported, e.g. `"United States"`.
    #[validate(length(min = 1))]
    pub country: String,
    /// Percentage weight of the country in the portfolio.
    pub weight_pct: Option<f64>,
}

/// An ETF that holds a given underlying equity (reverse exposure).
///
/// Standardizes `etf/equity_exposure` (FMP `etf-stock-exposure`). One row = one
/// fund holding the queried stock. `weight_pct` and `shares` are [`Option`]
/// because the exposure feed reports a variable subset per fund.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EtfEquityExposure {
    /// The underlying equity symbol the exposure was queried for.
    #[validate(length(min = 1))]
    pub equity_symbol: String,
    /// The ETF ticker holding the equity.
    #[validate(length(min = 1))]
    pub fund_symbol: String,
    /// Percentage weight of the equity within the fund.
    pub weight_pct: Option<f64>,
    /// Number of shares of the equity the fund holds.
    pub shares: Option<f64>,
    /// Market value of the position (listing currency).
    pub market_value: Option<f64>,
}

/// A US Treasury security auction result.
///
/// Standardizes `fixedincome/government/treasury_auctions` (US Treasury
/// `FiscalData` `securities/auctioned`). One row = one auctioned security. Most
/// rate / yield / amount fields are optional because the `FiscalData` record set
/// reports a different subset per security type (Bill / Note / Bond / TIPS /
/// FRN). Identity anchors are the CUSIP and the auction date.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct TreasuryAuction {
    /// CUSIP identifier of the auctioned security.
    #[validate(length(min = 1))]
    pub cusip: String,
    /// Security type, e.g. `"Bill"`, `"Note"`, `"Bond"`, `"TIPS"`, `"FRN"`.
    pub security_type: Option<String>,
    /// Security term as reported, e.g. `"10-Year"`, `"4-Week"`.
    pub security_term: Option<String>,
    /// Auction date.
    #[validate(length(min = 1))]
    pub auction_date: String,
    /// Issue (settlement) date.
    pub issue_date: Option<String>,
    /// Maturity date.
    pub maturity_date: Option<String>,
    /// High yield awarded at auction (percent), where applicable.
    pub high_yield: Option<f64>,
    /// Interest / coupon rate (percent), where applicable.
    pub interest_rate: Option<f64>,
    /// Total amount offered (USD).
    pub offering_amount: Option<f64>,
    /// Bid-to-cover ratio.
    pub bid_to_cover_ratio: Option<f64>,
}

/// A US Treasury security daily reference price.
///
/// Standardizes `fixedincome/government/treasury_prices` (US Treasury
/// `FiscalData` `accounting/od/avg_interest_rates` adjacent price datasets). One
/// row = one security's reference price on a given date. `price` is optional
/// for rows that report only reference metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct TreasuryPrice {
    /// CUSIP identifier of the security.
    #[validate(length(min = 1))]
    pub cusip: String,
    /// Security type, e.g. `"Bill"`, `"Note"`, `"Bond"`, `"TIPS"`.
    pub security_type: Option<String>,
    /// Pricing / observation date.
    #[validate(length(min = 1))]
    pub date: String,
    /// Reference (end-of-day) price.
    pub price: Option<f64>,
    /// Coupon rate (percent) where reported.
    pub coupon_rate: Option<f64>,
    /// Maturity date.
    pub maturity_date: Option<String>,
}

/// An FOMC document index entry.
///
/// Standardizes `regulators/fed/fomc_documents` (Federal Reserve FOMC document
/// listing). One row = one published FOMC document (statement, minutes,
/// projection, transcript). `url` and `date` are optional because the listing
/// surfaces a variable subset per document type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FomcDocument {
    /// Document type, e.g. `"statement"`, `"minutes"`, `"projection"`.
    #[validate(length(min = 1))]
    pub doc_type: String,
    /// Meeting / publication date.
    pub date: Option<String>,
    /// Document title.
    pub title: Option<String>,
    /// Source URL of the document.
    pub url: Option<String>,
}

/// Period price-performance row (1-day / 1-week / 1-month / 3-month / YTD / 1-year).
///
/// Standardizes `equity/price/performance`: the period total-return figures for a
/// single symbol. Each value is a fractional return (e.g. `0.012` is `+1.2%`);
/// fields are [`Option`] because provider coverage of each period varies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct PricePerformance {
    /// Underlying symbol.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Most-recent price used to anchor the period returns.
    pub price: Option<f64>,
    /// One-day return (fraction).
    pub one_day: Option<f64>,
    /// One-week (five-day) return (fraction).
    pub one_week: Option<f64>,
    /// One-month return (fraction).
    pub one_month: Option<f64>,
    /// Three-month return (fraction).
    pub three_month: Option<f64>,
    /// Year-to-date return (fraction).
    pub ytd: Option<f64>,
    /// One-year (fifty-two-week) return (fraction).
    pub one_year: Option<f64>,
}

/// A single equity-screener result row.
///
/// Standardizes `equity/screener` (FMP `stock-screener`): one row = one company
/// matched by the screener's filters. `symbol` is the identity anchor; the
/// descriptive and numeric attributes are [`Option`] because the screener
/// reports a different subset per company (e.g. ETFs omit fundamentals).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ScreenerRow {
    /// Ticker symbol (upper-cased by the provider fetcher).
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Company name as reported by the screener.
    pub company_name: Option<String>,
    /// Market capitalisation in the listing currency.
    pub market_cap: Option<f64>,
    /// Sector classification, e.g. `"Technology"`.
    pub sector: Option<String>,
    /// Industry classification, e.g. `"Consumer Electronics"`.
    pub industry: Option<String>,
    /// Beta versus the market.
    pub beta: Option<f64>,
    /// Most-recent share price.
    pub price: Option<f64>,
    /// Last annual dividend per share.
    pub last_annual_dividend: Option<f64>,
    /// Most-recent trading volume.
    pub volume: Option<f64>,
    /// Full exchange name, e.g. `"NASDAQ Global Select"`.
    pub exchange: Option<String>,
    /// Short exchange code, e.g. `"NASDAQ"`.
    pub exchange_short_name: Option<String>,
    /// ISO 3166-1 alpha-2 country code where the issuer is domiciled.
    pub country: Option<String>,
    /// Whether the security is an ETF.
    pub is_etf: Option<bool>,
    /// Whether the security is actively trading.
    pub is_actively_trading: Option<bool>,
}

/// One point on a futures forward curve: a contract and its last price.
///
/// Standardizes `derivatives/futures/curve`: one row per expiry along a root's
/// forward curve. `price`/`expiration` are [`Option`] because providers do not
/// always report a last price or a normalized expiry for every contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FuturesCurvePoint {
    /// Root / underlying futures symbol the curve belongs to.
    #[validate(length(min = 1))]
    pub underlying: String,
    /// Contract symbol for this expiry.
    #[validate(length(min = 1))]
    pub contract_symbol: String,
    /// Last traded price for the contract.
    pub price: Option<f64>,
    /// Contract expiry date in `YYYY-MM-DD` form when the provider reports it.
    pub expiration: Option<String>,
}

/// A company key executive / management-team member.
///
/// Standardizes `equity/fundamental/management` (FMP `key-executives`). One row =
/// one named executive. `name` and `symbol` are the identity anchors; the
/// descriptive and numeric attributes are [`Option`] because the source reports a
/// variable subset per officer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct KeyExecutive {
    /// Ticker symbol the executive is associated with.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Executive name.
    #[validate(length(min = 1))]
    pub name: String,
    /// Title / role, e.g. `"Chief Executive Officer"`.
    pub title: Option<String>,
    /// Reported total pay for the most-recent year, in the issuer's currency.
    pub pay: Option<f64>,
    /// Reporting currency (ISO 4217) for `pay` where reported.
    pub currency: Option<String>,
    /// Gender as reported by the source.
    pub gender: Option<String>,
    /// Year of birth where reported.
    pub year_born: Option<i32>,
    /// Title-since year where reported.
    pub title_since: Option<i32>,
}

/// An executive-compensation record for a single officer and fiscal year.
///
/// Standardizes `equity/fundamental/management_compensation` (FMP
/// `governance/executive_compensation`). One row = one (officer, fiscal year)
/// compensation disclosure. The pay components are [`Option`] because filers
/// disclose a different subset per officer/year.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ExecutiveCompensation {
    /// Ticker symbol the compensation record belongs to.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Officer name.
    #[validate(length(min = 1))]
    pub name_and_position: String,
    /// Fiscal year the compensation covers.
    pub fiscal_year: Option<i32>,
    /// Filing / acceptance date where reported.
    pub filing_date: Option<String>,
    /// Base salary.
    pub salary: Option<f64>,
    /// Cash bonus.
    pub bonus: Option<f64>,
    /// Value of stock awards.
    pub stock_award: Option<f64>,
    /// Value of option awards.
    pub option_award: Option<f64>,
    /// Non-equity incentive-plan compensation.
    pub incentive_plan_compensation: Option<f64>,
    /// All other compensation.
    pub all_other_compensation: Option<f64>,
    /// Total compensation across all components.
    pub total: Option<f64>,
    /// Reporting currency (ISO 4217) where reported.
    pub currency: Option<String>,
}

/// One revenue segment (business product line or geographic region) for a period.
///
/// Standardizes `equity/fundamental/revenue_per_segment` and
/// `equity/fundamental/revenue_per_geography` (FMP `revenue-product-segmentation`
/// / `revenue-geographic-segmentation`). One row = one (period, segment)
/// breakdown. `kind` discriminates product vs geographic; `revenue` is [`Option`]
/// because some periods report a segment with no value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct RevenueSegment {
    /// Ticker symbol the segment belongs to.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Segment variant: `"product"` or `"geography"`.
    #[validate(length(min = 1))]
    pub kind: String,
    /// Period end date the breakdown covers (`YYYY-MM-DD`).
    #[validate(length(min = 1))]
    pub date: String,
    /// Segment / region name, e.g. `"iPhone"`, `"Americas"`.
    #[validate(length(min = 1))]
    pub segment: String,
    /// Revenue attributed to the segment for the period.
    pub revenue: Option<f64>,
}

/// An earnings-call transcript for a single fiscal quarter.
///
/// Standardizes `equity/fundamental/transcript` (FMP `earning_call_transcript`).
/// One row = one (symbol, year, quarter) call transcript. `content` carries the
/// full transcript text; `date` and the period fields are [`Option`] because the
/// source occasionally omits them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EarningsTranscript {
    /// Ticker symbol the transcript belongs to.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Fiscal year of the call.
    pub year: Option<i32>,
    /// Fiscal quarter of the call (1-4).
    pub quarter: Option<i32>,
    /// Call date / timestamp where reported.
    pub date: Option<String>,
    /// Full transcript text.
    #[validate(length(min = 1))]
    pub content: String,
}

/// An environmental-social-governance (ESG) score observation.
///
/// Standardizes `equity/fundamental/esg_score` (FMP
/// `esg-environmental-social-governance-data`). One row = one (symbol, date) ESG
/// disclosure. The component and overall scores are [`Option`] because coverage
/// varies by issuer and period.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EsgScore {
    /// Ticker symbol the score belongs to.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Observation / acceptance date (`YYYY-MM-DD`).
    #[validate(length(min = 1))]
    pub date: String,
    /// Company name where reported.
    pub company_name: Option<String>,
    /// Environmental pillar score.
    pub environmental_score: Option<f64>,
    /// Social pillar score.
    pub social_score: Option<f64>,
    /// Governance pillar score.
    pub governance_score: Option<f64>,
    /// Overall ESG score.
    pub esg_score: Option<f64>,
}

/// A historical employee-headcount observation.
///
/// Standardizes `equity/fundamental/employee_count` (FMP
/// `historical/employee_count`). One row = one (symbol, period) headcount filing.
/// `employee_count` is [`Option`] because a filing may report only the filing
/// metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EmployeeCount {
    /// Ticker symbol the headcount belongs to.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Period-of-report date (`YYYY-MM-DD`).
    #[validate(length(min = 1))]
    pub period_of_report: String,
    /// Filing date where reported.
    pub filing_date: Option<String>,
    /// Reported full-time employee count.
    pub employee_count: Option<i64>,
    /// Source filing URL where reported.
    pub source: Option<String>,
}

/// A single company SEC-filing index entry.
///
/// Standardizes `equity/fundamental/filings` (FMP `sec_filings`). One row = one
/// filing in the company's index. `accepted_date`/`filing_date`/`link` are
/// [`Option`] because the index reports a variable subset per filing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CompanyFiling {
    /// Ticker symbol the filing belongs to.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// SEC form type, e.g. `"10-K"`, `"8-K"`.
    #[validate(length(min = 1))]
    pub form_type: String,
    /// Filing date (`YYYY-MM-DD`).
    pub filing_date: Option<String>,
    /// Acceptance date/time where reported.
    pub accepted_date: Option<String>,
    /// Central Index Key (CIK) where reported.
    pub cik: Option<String>,
    /// Direct link to the filing document/index.
    pub link: Option<String>,
    /// Final filing-document link where reported.
    pub final_link: Option<String>,
}

/// A single historical market-capitalization observation.
///
/// Standardizes `equity/historical_market_cap` (FMP
/// `historical-market-capitalization`). One row = one (symbol, date) market-cap
/// point. `market_cap` is [`Option`] because a date may report only metadata.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct HistoricalMarketCap {
    /// Ticker symbol the observation belongs to.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Observation date (`YYYY-MM-DD`).
    #[validate(length(min = 1))]
    pub date: String,
    /// Market capitalization in the listing currency.
    pub market_cap: Option<f64>,
}

/// A single reported company fact (an XBRL concept value for a period).
///
/// Standardizes `equity/compare/company_facts` (SEC
/// `api/xbrl/companyfacts/CIK*.json`). One row = one (concept, period) value the
/// filer reported. The taxonomy/concept/unit identify the fact; `value` and the
/// period anchors are [`Option`] because the SEC fact set reports a variable
/// subset per concept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CompanyFacts {
    /// Central Index Key (CIK) the facts belong to, unpadded.
    #[validate(length(min = 1))]
    pub cik: String,
    /// Entity (company) name where reported.
    pub entity_name: Option<String>,
    /// XBRL taxonomy the concept comes from, e.g. `"us-gaap"`, `"dei"`.
    #[validate(length(min = 1))]
    pub taxonomy: String,
    /// XBRL concept / tag, e.g. `"Revenues"`, `"Assets"`.
    #[validate(length(min = 1))]
    pub concept: String,
    /// Unit of measure, e.g. `"USD"`, `"shares"`.
    pub unit: Option<String>,
    /// Period end date the fact covers (`YYYY-MM-DD`).
    pub end: Option<String>,
    /// Fiscal year the fact is tagged to.
    pub fiscal_year: Option<i32>,
    /// Fiscal period the fact is tagged to, e.g. `"FY"`, `"Q1"`.
    pub fiscal_period: Option<String>,
    /// Source form the fact was reported on, e.g. `"10-K"`, `"10-Q"`.
    pub form: Option<String>,
    /// Reported numeric value of the fact.
    pub value: Option<f64>,
}

/// A single tradable derivatives instrument.
///
/// Standardizes `derivatives/futures/instruments` (the list of tradable
/// contracts) and `derivatives/futures/info` (metadata for one contract), both
/// Deribit-backed. One row = one instrument. The optional fields cover the
/// option-specific attributes (`strike`, `option_type`) that futures rows omit,
/// and the expiry timestamp that perpetuals omit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FuturesInstrument {
    /// Exchange instrument name, e.g. `"BTC-PERPETUAL"`, `"BTC-27DEC24"`.
    #[validate(length(min = 1))]
    pub instrument_name: String,
    /// Instrument kind as reported by the venue, e.g. `"future"`, `"option"`,
    /// `"future_combo"`.
    #[validate(length(min = 1))]
    pub kind: String,
    /// Base currency / settlement asset, e.g. `"BTC"`, `"ETH"`.
    pub currency: Option<String>,
    /// Strike price for option instruments (absent for futures/perpetuals).
    pub strike: Option<f64>,
    /// Expiry as a Unix timestamp in milliseconds, when the contract expires.
    pub expiration_timestamp: Option<u64>,
    /// Option type (`"call"` / `"put"`) for option instruments.
    pub option_type: Option<String>,
    /// Whether the instrument is currently active / tradable.
    pub is_active: Option<bool>,
}

/// A SEC-regulated institution discovery row.
///
/// Standardizes `regulators/sec/institutions_search` (filtering SEC's public
/// `company_tickers.json` directory by name). One row = one institution / issuer
/// matched by the query. The CIK and reported name are the identity anchors; the
/// ticker is [`Option`] because not every filer in the directory lists one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SecInstitution {
    /// Central Index Key (CIK) as reported by SEC, unpadded.
    #[validate(length(min = 1))]
    pub cik: String,
    /// Institution / issuer name as reported in the directory.
    #[validate(length(min = 1))]
    pub name: String,
    /// Exchange ticker symbol where the directory supplies one.
    pub symbol: Option<String>,
}

/// A Standard Industrial Classification (SIC) industry-code row.
///
/// Standardizes `regulators/sec/sic_search` (filtering the SEC-published SIC
/// code list by query). One row = one (code, description) pair. The office is the
/// SEC Division of Corporation Finance industry-review office, present for most
/// codes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct SicCode {
    /// Four-digit SIC code, e.g. `"3571"`.
    #[validate(length(min = 1))]
    pub code: String,
    /// Industry-title description for the code.
    #[validate(length(min = 1))]
    pub description: String,
    /// SEC industry-review office responsible for the code, where known.
    pub office: Option<String>,
}

/// Filing header metadata for a single EDGAR accession.
///
/// Standardizes `regulators/sec/filing_headers` (the EDGAR
/// `{accession}-index.json` header block). One row = one filing. The CIK and
/// accession number are the identity anchors; the remaining descriptive fields
/// are [`Option`] because the index header populates a variable subset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FilingHeader {
    /// Filer Central Index Key (CIK), unpadded.
    #[validate(length(min = 1))]
    pub cik: String,
    /// Accession number, e.g. `"0000320193-24-000123"`.
    #[validate(length(min = 1))]
    pub accession_number: String,
    /// Form type, e.g. `"10-K"`, `"8-K"`, where reported.
    pub form_type: Option<String>,
    /// Filing / acceptance date (`YYYY-MM-DD`) where reported.
    pub filing_date: Option<String>,
    /// Period of report (`YYYY-MM-DD`) where reported.
    pub period_of_report: Option<String>,
    /// Human-readable filing description where reported.
    pub description: Option<String>,
}

/// A single file listed within an EDGAR filing's document index.
///
/// Standardizes `regulators/sec/schema_files` (the `directory.item[]` array of
/// the EDGAR `{accession}-index.json`). One row = one document/schema file in the
/// filing. The file name is the identity anchor; size, type, and last-modified
/// are [`Option`] since the index reports a variable subset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct FilingFile {
    /// File name within the filing directory, e.g. `"aapl-20240928.htm"`.
    #[validate(length(min = 1))]
    pub name: String,
    /// Document type label where reported, e.g. `"10-K"`, `"EX-101.SCH"`.
    pub file_type: Option<String>,
    /// File size in bytes where reported.
    pub size: Option<u64>,
    /// Last-modified timestamp where reported.
    pub last_modified: Option<String>,
}

/// A single SEC litigation-release item from the litigation RSS feed.
///
/// Standardizes `regulators/sec/rss_litigation` (the SEC litigation-releases RSS
/// feed). One row = one release. The title and link are the identity anchors; the
/// publication date and summary are [`Option`] because feed items vary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct LitigationRelease {
    /// Release title.
    #[validate(length(min = 1))]
    pub title: String,
    /// Link to the litigation-release document.
    #[validate(length(min = 1))]
    pub link: String,
    /// Publication date as reported in the feed.
    pub published: Option<String>,
    /// Short summary / description of the release.
    pub summary: Option<String>,
}

/// A reported short-interest record.
///
/// Standardizes `equity/shorts/short_interest` (FINRA consolidated short
/// interest). One row = one issuer's bi-monthly short-interest report: the
/// current and previous settlement-cycle short-interest counts, the percent
/// change between them, the average daily share volume, and the days-to-cover
/// ratio. `current_short_interest`/`previous_short_interest`/
/// `avg_daily_share_volume` are share counts; `percent_change` and
/// `days_to_cover` are ratios as reported by FINRA.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct ShortInterest {
    /// Issuer / company name.
    #[validate(length(min = 1))]
    pub issuer_name: String,
    /// Ticker symbol the record concerns.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// FINRA market-class code (e.g. `"G"`).
    pub market_class_code: Option<String>,
    /// Reported short interest for the current settlement cycle (share count).
    pub current_short_interest: Option<i64>,
    /// Reported short interest for the previous settlement cycle (share count).
    pub previous_short_interest: Option<i64>,
    /// Percent change in short interest versus the previous cycle.
    pub percent_change: Option<f64>,
    /// Average daily share volume over the cycle.
    pub avg_daily_share_volume: Option<i64>,
    /// Days-to-cover (short interest divided by average daily volume).
    pub days_to_cover: Option<f64>,
    /// Settlement date the cycle reports for, in `YYYY-MM-DD` form.
    pub settlement_date: Option<String>,
}

/// A weekly OTC / dark-pool market-volume record.
///
/// Standardizes `equity/darkpool/otc` (FINRA weekly OTC transparency / ATS &
/// non-ATS summary). One row = one market participant's weekly aggregate over-
/// the-counter trading: the total share quantity and trade count it reported for
/// the trade-report week.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct OtcMarketVolume {
    /// Trade-report week start date, in `YYYY-MM-DD` form.
    #[validate(length(min = 1))]
    pub trade_report_date: String,
    /// FINRA market-participant identifier (MPID).
    #[validate(length(min = 1))]
    pub market_participant_identifier: String,
    /// Total share quantity the participant reported for the week.
    pub total_share_quantity: Option<i64>,
    /// Total trade count the participant reported for the week.
    pub total_trade_count: Option<i64>,
}

/// A single constituent (member security) of a market index.
///
/// Standardizes `index/constituents` (FMP `/{index}_constituent`). One row =
/// one (index, member) pair. The index handle and the member symbol are the
/// identity anchors; the descriptive fields are [`Option`] because the
/// constituent feed reports a variable subset per index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct IndexConstituent {
    /// Index handle the membership was queried for, e.g. `"sp500"`.
    #[validate(length(min = 1))]
    pub index: String,
    /// Constituent ticker symbol.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Constituent company / security name as reported.
    pub name: Option<String>,
    /// GICS / index sector label, where reported.
    pub sector: Option<String>,
    /// GICS / index sub-industry label, where reported.
    pub sub_industry: Option<String>,
    /// Headquarters location, where reported.
    pub headquarters: Option<String>,
    /// Date the security was added to the index (`YYYY-MM-DD`), where reported.
    pub date_added: Option<String>,
}

/// A single Shiller / S&P 500 valuation-multiple observation.
///
/// Standardizes `index/sp500_multiples` (Nasdaq Data Link `MULTPL/*`, e.g. the
/// Shiller CAPE PE ratio). One row = one (metric, date) observation. The metric
/// label and the date are the identity anchors; `value` is [`Option`] because a
/// source cell may be blank for a given period.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Sp500Multiple {
    /// Multiple name, e.g. `"shiller_pe_ratio_month"`.
    #[validate(length(min = 1))]
    pub metric: String,
    /// Observation date in `YYYY-MM-DD` form.
    #[validate(length(min = 1))]
    pub date: String,
    /// Observed value of the multiple for the date.
    pub value: Option<f64>,
}

/// A foreign-exchange (FX) pair last-price snapshot.
///
/// Standardizes `currency/snapshots` (FMP `/fx` forex snapshot). One row = one
/// pair. The `symbol` (a `BASE/QUOTE` ticker) is the identity anchor; the
/// price/bid/ask fields are [`Option`] because the forex snapshot reports a
/// variable subset per pair.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CurrencySnapshot {
    /// FX-pair ticker, e.g. `"EUR/USD"`.
    #[validate(length(min = 1))]
    pub symbol: String,
    /// Base currency code (left side of the pair), where reported.
    pub base_currency: Option<String>,
    /// Quote currency code (right side of the pair), where reported.
    pub quote_currency: Option<String>,
    /// Most-recent mid / last price.
    pub price: Option<f64>,
    /// Best bid price.
    pub bid: Option<f64>,
    /// Best ask price.
    pub ask: Option<f64>,
    /// Absolute price change over the session.
    pub change: Option<f64>,
    /// Relative price change as a percentage.
    pub change_percent: Option<f64>,
    /// Snapshot timestamp as Unix epoch milliseconds (UTC), where reported.
    pub ts_ms: Option<i64>,
}

/// A single standardized company data-point / attribute observation.
///
/// Standardizes the `equity/fundamental/historical_attributes`,
/// `equity/fundamental/latest_attributes`, and `equity/fundamental/search_attributes`
/// cluster (`Intrinio`-backed). Intrinio exposes thousands of standardized
/// "data-point" tags (e.g. `marketcap`, `pricetoearnings`, `ebitda`) reachable
/// individually per company or as a historical series; one row = one
/// (symbol, tag, date) observation, or one matched tag for the search variant.
/// Numeric and descriptive fields are [`Option`] since the latest-point and
/// metadata-search variants populate different subsets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct CompanyAttribute {
    /// Company identifier the attribute belongs to (ticker / Intrinio id).
    /// Empty for the metadata `search_attributes` variant, which describes a
    /// tag rather than a company observation.
    pub symbol: Option<String>,
    /// Standardized data-point tag, e.g. `"marketcap"`, `"pricetoearnings"`.
    #[validate(length(min = 1))]
    pub tag: String,
    /// Human-readable tag name, where the API reports it (search/metadata).
    pub name: Option<String>,
    /// Observation date for a (historical) attribute series, `YYYY-MM-DD`.
    pub date: Option<String>,
    /// Numeric attribute value, when the tag is numeric.
    pub value: Option<f64>,
    /// Textual attribute value, when the tag is non-numeric.
    pub text_value: Option<String>,
    /// Unit / type label for the tag, where reported, e.g. `"usd"`, `"ratio"`.
    pub unit: Option<String>,
    /// Statement type / category the tag belongs to, where reported.
    pub statement_code: Option<String>,
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
    fn portfolio_return_round_trips_and_validates() {
        let row = PortfolioReturn {
            date: "2024-06-03".to_string(),
            portfolio: "SMALL LoBM".to_string(),
            value: Some(0.0123),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let absent = PortfolioReturn {
            value: None,
            ..row.clone()
        };
        assert!(absent.validate().is_ok());
        round_trip(&absent);

        let bad = PortfolioReturn {
            portfolio: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn breakpoint_round_trips_and_validates() {
        let row = Breakpoint {
            date: "2024-06".to_string(),
            breakpoint: "95".to_string(),
            value: Some(12.5),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let bad = Breakpoint {
            date: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn imf_discovery_record_round_trips_and_validates() {
        let row = ImfDiscoveryRecord {
            kind: "dataflow".to_string(),
            id: "IFS".to_string(),
            name: Some("International Financial Statistics".to_string()),
            dataflow: Some("IFS".to_string()),
            structure: Some("ECOFIN_DSD".to_string()),
            position: None,
            value: None,
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let dimension = ImfDiscoveryRecord {
            kind: "dimension".to_string(),
            id: "FREQ".to_string(),
            name: Some("Frequency".to_string()),
            dataflow: Some("IFS".to_string()),
            structure: Some("ECOFIN_DSD".to_string()),
            position: Some(1),
            value: None,
        };
        assert!(dimension.validate().is_ok());
        round_trip(&dimension);

        let bad = ImfDiscoveryRecord {
            id: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn company_attribute_round_trips_and_validates() {
        let historical = CompanyAttribute {
            symbol: Some("AAPL".to_string()),
            tag: "marketcap".to_string(),
            name: Some("Market Capitalization".to_string()),
            date: Some("2024-09-28".to_string()),
            value: Some(3_400_000_000_000.0),
            text_value: None,
            unit: Some("usd".to_string()),
            statement_code: None,
        };
        assert!(historical.validate().is_ok());
        round_trip(&historical);

        // The metadata `search_attributes` variant carries no symbol/value.
        let search = CompanyAttribute {
            symbol: None,
            tag: "pricetoearnings".to_string(),
            name: Some("Price to Earnings".to_string()),
            date: None,
            value: None,
            text_value: None,
            unit: Some("ratio".to_string()),
            statement_code: Some("calculations".to_string()),
        };
        assert!(search.validate().is_ok());
        round_trip(&search);

        // An empty tag is the one required anchor and must be rejected.
        let bad = CompanyAttribute {
            tag: String::new(),
            ..historical
        };
        assert!(bad.validate().is_err());
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
    fn price_performance_round_trips_and_validates() {
        let perf = PricePerformance {
            symbol: "AAPL".to_string(),
            price: Some(202.0),
            one_day: Some(0.012),
            one_week: None,
            one_month: Some(0.034),
            three_month: Some(0.08),
            ytd: Some(0.15),
            one_year: Some(0.27),
        };
        assert!(perf.validate().is_ok());
        round_trip(&perf);

        let bad = PricePerformance {
            symbol: String::new(),
            ..perf
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn screener_row_round_trips_and_validates() {
        let row = ScreenerRow {
            symbol: "AAPL".to_string(),
            company_name: Some("Apple Inc.".to_string()),
            market_cap: Some(3.45e12),
            sector: Some("Technology".to_string()),
            industry: Some("Consumer Electronics".to_string()),
            beta: Some(1.24),
            price: Some(202.0),
            last_annual_dividend: Some(0.99),
            volume: Some(55_000_000.0),
            exchange: Some("NASDAQ Global Select".to_string()),
            exchange_short_name: Some("NASDAQ".to_string()),
            country: Some("US".to_string()),
            is_etf: Some(false),
            is_actively_trading: Some(true),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let bad = ScreenerRow {
            symbol: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn etf_info_round_trips_and_validates() {
        let info = EtfInfo {
            symbol: "SPY".to_string(),
            name: "SPDR S&P 500 ETF Trust".to_string(),
            issuer: Some("State Street".to_string()),
            nav: Some(540.12),
            aum: Some(5.2e11),
            expense_ratio: Some(0.000_945),
            holdings_count: Some(503),
            currency: Some("USD".to_string()),
            exchange: Some("NYSE Arca".to_string()),
            inception_date: Some("1993-01-22".to_string()),
            description: Some("Tracks the S&P 500 index.".to_string()),
        };
        assert!(info.validate().is_ok());
        round_trip(&info);

        let bad = EtfInfo {
            name: String::new(),
            ..info
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn etf_country_weight_round_trips_and_validates() {
        let weight = EtfCountryWeight {
            fund_symbol: "SPY".to_string(),
            country: "United States".to_string(),
            weight_pct: Some(98.7),
        };
        assert!(weight.validate().is_ok());
        round_trip(&weight);

        let bad = EtfCountryWeight {
            country: String::new(),
            ..weight
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn etf_equity_exposure_round_trips_and_validates() {
        let exposure = EtfEquityExposure {
            equity_symbol: "AAPL".to_string(),
            fund_symbol: "SPY".to_string(),
            weight_pct: Some(7.1),
            shares: Some(1_234_567.0),
            market_value: Some(2.5e8),
        };
        assert!(exposure.validate().is_ok());
        round_trip(&exposure);

        let bad = EtfEquityExposure {
            fund_symbol: String::new(),
            ..exposure
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn futures_curve_point_round_trips_and_validates() {
        let point = FuturesCurvePoint {
            underlying: "ES=F".to_string(),
            contract_symbol: "ESM26.CME".to_string(),
            price: Some(5300.0),
            expiration: Some("2026-06-19".to_string()),
        };
        assert!(point.validate().is_ok());
        round_trip(&point);

        let bad = FuturesCurvePoint {
            contract_symbol: String::new(),
            ..point
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
    fn commitment_of_traders_round_trips_and_validates() {
        let cot = CommitmentOfTraders {
            market: "WHEAT-SRW - CHICAGO BOARD OF TRADE".to_string(),
            report_date: "2024-06-04".to_string(),
            open_interest: Some(420_000.0),
            noncommercial_long: Some(95_000.0),
            noncommercial_short: Some(120_000.0),
            commercial_long: Some(200_000.0),
            commercial_short: Some(180_000.0),
            total_reportable_long: Some(360_000.0),
            total_reportable_short: Some(340_000.0),
            nonreportable_long: Some(60_000.0),
            nonreportable_short: Some(80_000.0),
        };
        assert!(cot.validate().is_ok());
        round_trip(&cot);

        let bad = CommitmentOfTraders {
            market: String::new(),
            ..cot
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
    fn commodity_report_row_round_trips_and_validates() {
        let row = CommodityReportRow {
            report: "petroleum_status_report".to_string(),
            series_id: "WCRSTUS1".to_string(),
            period: "2026-05-30".to_string(),
            series_description: Some("Weekly U.S. Ending Stocks of Crude Oil".to_string()),
            value: Some(440_123.0),
            units: Some("thousand barrels".to_string()),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let missing = CommodityReportRow {
            value: None,
            ..row.clone()
        };
        assert!(missing.validate().is_ok());
        round_trip(&missing);

        let bad = CommodityReportRow {
            series_id: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn calendar_event_round_trips_and_validates() {
        let dividend = CalendarEvent {
            kind: "dividend".to_string(),
            symbol: "AAPL".to_string(),
            name: Some("Apple Inc.".to_string()),
            date: Some("2026-05-12".to_string()),
            dividend: Some(0.25),
            payment_date: Some("2026-05-15".to_string()),
            record_date: Some("2026-05-13".to_string()),
            eps_estimate: None,
            fiscal_period: None,
            price: None,
            shares: None,
            exchange: None,
        };
        assert!(dividend.validate().is_ok());
        round_trip(&dividend);

        let bad = CalendarEvent {
            symbol: String::new(),
            ..dividend
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

    #[test]
    fn key_executive_round_trips_and_validates() {
        let exec = KeyExecutive {
            symbol: "AAPL".to_string(),
            name: "Tim Cook".to_string(),
            title: Some("Chief Executive Officer".to_string()),
            pay: Some(16_000_000.0),
            currency: Some("USD".to_string()),
            gender: Some("male".to_string()),
            year_born: Some(1960),
            title_since: Some(2011),
        };
        assert!(exec.validate().is_ok());
        round_trip(&exec);

        let bad = KeyExecutive {
            name: String::new(),
            ..exec
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn executive_compensation_round_trips_and_validates() {
        let comp = ExecutiveCompensation {
            symbol: "AAPL".to_string(),
            name_and_position: "Tim Cook CEO".to_string(),
            fiscal_year: Some(2024),
            filing_date: Some("2025-01-05".to_string()),
            salary: Some(3_000_000.0),
            bonus: Some(0.0),
            stock_award: Some(50_000_000.0),
            option_award: Some(0.0),
            incentive_plan_compensation: Some(12_000_000.0),
            all_other_compensation: Some(1_500_000.0),
            total: Some(66_500_000.0),
            currency: Some("USD".to_string()),
        };
        assert!(comp.validate().is_ok());
        round_trip(&comp);

        let bad = ExecutiveCompensation {
            name_and_position: String::new(),
            ..comp
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn revenue_segment_round_trips_and_validates() {
        let seg = RevenueSegment {
            symbol: "AAPL".to_string(),
            kind: "product".to_string(),
            date: "2024-09-28".to_string(),
            segment: "iPhone".to_string(),
            revenue: Some(201_183_000_000.0),
        };
        assert!(seg.validate().is_ok());
        round_trip(&seg);

        let bad = RevenueSegment {
            segment: String::new(),
            ..seg
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn earnings_transcript_round_trips_and_validates() {
        let transcript = EarningsTranscript {
            symbol: "AAPL".to_string(),
            year: Some(2024),
            quarter: Some(4),
            date: Some("2024-10-31 17:00:00".to_string()),
            content: "Operator: Good afternoon...".to_string(),
        };
        assert!(transcript.validate().is_ok());
        round_trip(&transcript);

        let bad = EarningsTranscript {
            content: String::new(),
            ..transcript
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn esg_score_round_trips_and_validates() {
        let score = EsgScore {
            symbol: "AAPL".to_string(),
            date: "2024-09-28".to_string(),
            company_name: Some("Apple Inc.".to_string()),
            environmental_score: Some(72.5),
            social_score: Some(55.0),
            governance_score: Some(61.0),
            esg_score: Some(62.8),
        };
        assert!(score.validate().is_ok());
        round_trip(&score);

        let bad = EsgScore {
            date: String::new(),
            ..score
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn employee_count_round_trips_and_validates() {
        let count = EmployeeCount {
            symbol: "AAPL".to_string(),
            period_of_report: "2024-09-28".to_string(),
            filing_date: Some("2024-11-01".to_string()),
            employee_count: Some(164_000),
            source: Some("https://www.sec.gov/...".to_string()),
        };
        assert!(count.validate().is_ok());
        round_trip(&count);

        let bad = EmployeeCount {
            period_of_report: String::new(),
            ..count
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn company_filing_round_trips_and_validates() {
        let filing = CompanyFiling {
            symbol: "AAPL".to_string(),
            form_type: "10-K".to_string(),
            filing_date: Some("2024-11-01".to_string()),
            accepted_date: Some("2024-11-01 18:00:00".to_string()),
            cik: Some("0000320193".to_string()),
            link: Some("https://www.sec.gov/...".to_string()),
            final_link: Some("https://www.sec.gov/....htm".to_string()),
        };
        assert!(filing.validate().is_ok());
        round_trip(&filing);

        let bad = CompanyFiling {
            form_type: String::new(),
            ..filing
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn historical_market_cap_round_trips_and_validates() {
        let cap = HistoricalMarketCap {
            symbol: "AAPL".to_string(),
            date: "2024-09-28".to_string(),
            market_cap: Some(3.45e12),
        };
        assert!(cap.validate().is_ok());
        round_trip(&cap);

        let bad = HistoricalMarketCap {
            date: String::new(),
            ..cap
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn company_facts_round_trips_and_validates() {
        let fact = CompanyFacts {
            cik: "320193".to_string(),
            entity_name: Some("Apple Inc.".to_string()),
            taxonomy: "us-gaap".to_string(),
            concept: "Revenues".to_string(),
            unit: Some("USD".to_string()),
            end: Some("2024-09-28".to_string()),
            fiscal_year: Some(2024),
            fiscal_period: Some("FY".to_string()),
            form: Some("10-K".to_string()),
            value: Some(391_035_000_000.0),
        };
        assert!(fact.validate().is_ok());
        round_trip(&fact);

        let bad = CompanyFacts {
            concept: String::new(),
            ..fact
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn futures_instrument_round_trips_and_validates() {
        let inst = FuturesInstrument {
            instrument_name: "BTC-PERPETUAL".to_string(),
            kind: "future".to_string(),
            currency: Some("BTC".to_string()),
            strike: None,
            expiration_timestamp: None,
            option_type: None,
            is_active: Some(true),
        };
        assert!(inst.validate().is_ok());
        round_trip(&inst);

        let bad = FuturesInstrument {
            instrument_name: String::new(),
            ..inst
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn sec_institution_round_trips_and_validates() {
        let row = SecInstitution {
            cik: "320193".to_string(),
            name: "Apple Inc.".to_string(),
            symbol: Some("AAPL".to_string()),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let bad = SecInstitution {
            name: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn sic_code_round_trips_and_validates() {
        let row = SicCode {
            code: "3571".to_string(),
            description: "Electronic Computers".to_string(),
            office: Some("Office of Technology".to_string()),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let bad = SicCode {
            code: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn filing_header_round_trips_and_validates() {
        let row = FilingHeader {
            cik: "320193".to_string(),
            accession_number: "0000320193-24-000123".to_string(),
            form_type: Some("10-K".to_string()),
            filing_date: Some("2024-10-01".to_string()),
            period_of_report: Some("2024-09-28".to_string()),
            description: Some("Annual report".to_string()),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let bad = FilingHeader {
            accession_number: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn filing_file_round_trips_and_validates() {
        let row = FilingFile {
            name: "aapl-20240928.htm".to_string(),
            file_type: Some("10-K".to_string()),
            size: Some(1_234_567),
            last_modified: Some("2024-10-01 18:01:14".to_string()),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let bad = FilingFile {
            name: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn litigation_release_round_trips_and_validates() {
        let row = LitigationRelease {
            title: "SEC Charges Example Corp".to_string(),
            link: "https://www.sec.gov/litigation/litreleases/lr-12345".to_string(),
            published: Some("Mon, 03 Jun 2024 12:00:00 EST".to_string()),
            summary: Some("The SEC announced charges...".to_string()),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let bad = LitigationRelease {
            link: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn index_constituent_round_trips_and_validates() {
        let row = IndexConstituent {
            index: "sp500".to_string(),
            symbol: "AAPL".to_string(),
            name: Some("Apple Inc.".to_string()),
            sector: Some("Information Technology".to_string()),
            sub_industry: Some("Technology Hardware, Storage & Peripherals".to_string()),
            headquarters: Some("Cupertino, California".to_string()),
            date_added: Some("1982-11-30".to_string()),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let sparse = IndexConstituent {
            name: None,
            sector: None,
            sub_industry: None,
            headquarters: None,
            date_added: None,
            ..row.clone()
        };
        assert!(sparse.validate().is_ok());
        round_trip(&sparse);

        let bad = IndexConstituent {
            symbol: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn sp500_multiple_round_trips_and_validates() {
        let row = Sp500Multiple {
            metric: "shiller_pe_ratio_month".to_string(),
            date: "2024-06-01".to_string(),
            value: Some(34.21),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let absent = Sp500Multiple {
            value: None,
            ..row.clone()
        };
        assert!(absent.validate().is_ok());
        round_trip(&absent);

        let bad = Sp500Multiple {
            metric: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn currency_snapshot_round_trips_and_validates() {
        let row = CurrencySnapshot {
            symbol: "EUR/USD".to_string(),
            base_currency: Some("EUR".to_string()),
            quote_currency: Some("USD".to_string()),
            price: Some(1.0825),
            bid: Some(1.0824),
            ask: Some(1.0826),
            change: Some(0.0012),
            change_percent: Some(0.111),
            ts_ms: Some(1_717_200_000_000),
        };
        assert!(row.validate().is_ok());
        round_trip(&row);

        let sparse = CurrencySnapshot {
            base_currency: None,
            quote_currency: None,
            price: None,
            bid: None,
            ask: None,
            change: None,
            change_percent: None,
            ts_ms: None,
            ..row.clone()
        };
        assert!(sparse.validate().is_ok());
        round_trip(&sparse);

        let bad = CurrencySnapshot {
            symbol: String::new(),
            ..row
        };
        assert!(bad.validate().is_err());
    }
}
