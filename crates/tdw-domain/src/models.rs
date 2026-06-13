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
}
