//! Real FMP (Financial Modeling Prep) HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. The fetchers cover the market-data and
//! fundamentals clusters, each normalizing FMP's documented REST responses to a
//! standardized `tdw-domain` model:
//!
//!   - [`FmpHttpHistoricalFetcher`] — daily OHLCV bars (`/historical-price-full`)
//!   - [`FmpHttpQuoteSnapshotFetcher`] — last-price quote (`/quote`)
//!   - [`FmpHttpIncomeFetcher`] — legacy income rows (`/income-statement`)
//!   - [`FmpHttpStatementFetcher`] — balance / income / cash statements and their
//!     `*-growth` siblings, normalized to [`FinancialStatement`]
//!   - [`FmpHttpKeyMetricsFetcher`] — per-share & valuation metrics (`/key-metrics`)
//!   - [`FmpHttpRatiosFetcher`] — financial ratios (`/ratios`)
//!   - [`FmpHttpPeersFetcher`] — comparable tickers (`/stock_peers`)
//!   - [`FmpHttpProfileFetcher`] — company profile (`/profile`)
//!   - [`FmpHttpDividendsFetcher`] — historical dividends (`/historical-price-full/stock_dividend`)
//!   - [`FmpHttpSplitsFetcher`] — historical splits (`/historical-price-full/stock_split`)
//!   - [`FmpHttpEarningsFetcher`] — historical EPS (`/historical/earning_calendar`)
//!   - [`FmpHttpDiscoveryFetcher`] — market movers (`/stock_market/{gainers,losers,actives}`)
//!   - [`FmpHttpScreenerFetcher`] — equity screener (`/stock-screener`)
//!   - [`FmpHttpPriceTargetFetcher`] — analyst price-target consensus (`/v4/price-target-consensus`)
//!   - [`FmpHttpAnalystEstimatesFetcher`] — forward analyst estimates (`/analyst-estimates`)
//!
//! Live calls require `TDW_FMP_API_KEY`. The live integration test is
//! additionally gated by `TDW_FMP_LIVE=1` so unattended CI stays offline.

#![cfg(feature = "http")]

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Map;
use tdw_core::http_support::prelude::*;
use tdw_domain::{
    CalendarEvent, CompanyFiling, CompanyProfile, CorporateAction, EarningsTranscript,
    EmployeeCount, EsgScore, Estimate, ExecutiveCompensation, FinancialStatement,
    HistoricalMarketCap, Instrument, KeyExecutive, KeyMetrics, MarketDataBar, Ohlcv,
    OwnershipRecord, QuoteSnapshot, Ratios, RevenueSegment, ScreenerRow, StatementKind,
    TimeGranularity,
};

use crate::{
    API_KEY_ENV, BASE_URL, FmpCalendarRangeQuery, FmpDiscoveryDirection, FmpDiscoveryQuery,
    FmpError, FmpFundamentalQuery, FmpFundamentalsQuery, FmpHistoricalQuery, FmpIncomeRow,
    FmpLimitQuery, FmpQuoteQuery, FmpRevenueSegmentQuery, FmpScreenerQuery, FmpSearchQuery,
    FmpSegmentKind, FmpStatement, FmpStatementQuery, FmpSymbolLimitQuery, FmpSymbolQuery,
    FmpTranscriptQuery,
};

const USER_AGENT: &str = "tdw-provider-fmp/0.1";

// ---------------------------------------------------------------------------
// Internal serde shapes for the FMP API responses
// ---------------------------------------------------------------------------

/// Internal wire shape for `/quote/{symbol}` entries.
#[derive(Deserialize)]
struct FmpQuoteRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    price: f64,
    #[serde(default)]
    change: f64,
    #[serde(rename = "changesPercentage", default)]
    changes_percentage: f64,
    #[serde(rename = "previousClose", default)]
    previous_close: f64,
    #[serde(default)]
    timestamp: i64,
}

#[derive(Deserialize)]
struct FmpHistoricalEnvelope {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    historical: Vec<FmpHistoricalBar>,
}

#[derive(Deserialize)]
struct FmpHistoricalBar {
    date: String,
    #[serde(flatten)]
    ohlcv: Ohlcv,
}

#[derive(Deserialize)]
struct FmpIncomeStatementRaw {
    #[serde(default)]
    date: String,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    revenue: i64,
    #[serde(rename = "grossProfit", default)]
    gross_profit: i64,
    #[serde(rename = "netIncome", default)]
    net_income: i64,
}

// ---------------------------------------------------------------------------
// Helper: build a reqwest Client and read the API key
// ---------------------------------------------------------------------------

fn fmp_client() -> std::result::Result<Client, FmpError> {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| FmpError::Provider(format!("fmp client build: {e}")))
}

fn api_key() -> Result<String> {
    tdw_core::http_support::read_required_key(API_KEY_ENV, "fmp")
}

// ---------------------------------------------------------------------------
// FmpHttpHistoricalFetcher
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP daily-bar fetcher.
    pub FmpHttpHistoricalFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FmpHistoricalQuery, MarketDataBar> for FmpHttpHistoricalFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<FmpHistoricalQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("fmp symbol must be a string".to_string()))?;
        FmpHistoricalQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpHistoricalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = api_key()?;
        let url = format!(
            "{}/historical-price-full/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = fmp_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[("apikey", api_key.as_str())])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("fmp extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "fmp historical returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("fmp read body: {e}")))
    }

    fn transform_data(&self, query: &FmpHistoricalQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        let envelope: FmpHistoricalEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp parse_json: {e}")))?;
        let symbol = envelope.symbol.unwrap_or_else(|| query.symbol.clone());
        let mut rows = Vec::with_capacity(envelope.historical.len());
        for bar in envelope.historical {
            rows.push(MarketDataBar {
                symbol: symbol.clone(),
                venue: "fmp".to_string(),
                granularity: TimeGranularity::Day,
                ts: bar.date,
                source: "fmp".to_string(),
                ..bar.ohlcv.into_bar_template()
            });
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpIncomeFetcher
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP income-statement fetcher.
    pub FmpHttpIncomeFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FmpFundamentalsQuery, FmpIncomeRow> for FmpHttpIncomeFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "income_statement";

    fn transform_query(params: Value) -> Result<FmpFundamentalsQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("fmp symbol must be a string".to_string()))?;
        let statement_str = params
            .get("statement")
            .and_then(Value::as_str)
            .unwrap_or("income");
        let statement = match statement_str {
            "income" => FmpStatement::Income,
            "balance" => FmpStatement::Balance,
            "cashflow" => FmpStatement::Cashflow,
            other => {
                return Err(Error::InvalidQuery(format!(
                    "fmp unknown statement type: {other}"
                )));
            }
        };
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|e| Error::InvalidQuery(format!("fmp limit too large: {e}")))?
            .unwrap_or(5);
        FmpFundamentalsQuery::new(symbol, statement, limit)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpFundamentalsQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let api_key = api_key()?;
        let path_segment = query.statement.as_path_segment();
        let url = format!(
            "{}/{}/{}",
            self.base_url().trim_end_matches('/'),
            path_segment,
            query.symbol,
        );
        let client = fmp_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[("limit", query.limit.to_string()), ("apikey", api_key)])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("fmp income extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "fmp income-statement returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("fmp income read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &FmpFundamentalsQuery,
        raw: Bytes,
    ) -> Result<Vec<FmpIncomeRow>> {
        let statements: Vec<FmpIncomeStatementRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp income parse_json: {e}")))?;
        let rows = statements
            .into_iter()
            .map(|s| FmpIncomeRow {
                symbol: s.symbol.unwrap_or_else(|| query.symbol.clone()),
                date: s.date,
                revenue: s.revenue,
                gross_profit: s.gross_profit,
                net_income: s.net_income,
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpQuoteSnapshotFetcher
// ---------------------------------------------------------------------------

/// Production FMP last-price quote-snapshot fetcher.
///
/// Calls `/quote/{symbol}` (FMP free tier). Returns a single [`QuoteSnapshot`]
/// per symbol with the most-recent trade price, absolute/relative change
/// versus the previous close, and a millisecond-precision timestamp.
///
/// This is a **fresh-read path** — no caching or persistence is applied.
/// Results are intended for real-time consumers such as a price-alert engine
/// that must compare `current_price` to alert thresholds on every evaluation.
#[derive(Clone, Debug)]
pub struct FmpHttpQuoteSnapshotFetcher {
    base_url: String,
}

impl Default for FmpHttpQuoteSnapshotFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl FmpHttpQuoteSnapshotFetcher {
    /// Override the FMP base URL (useful for tests with a mock server).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry for the `fmp` / `quote_snapshot` slot.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<FmpQuoteQuery, QuoteSnapshot> for FmpHttpQuoteSnapshotFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "quote_snapshot";

    fn transform_query(params: Value) -> Result<FmpQuoteQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("fmp symbol must be a string".to_string()))?;
        FmpQuoteQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &FmpQuoteQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = api_key()?;
        let url = format!(
            "{}/quote/{}",
            self.base_url.trim_end_matches('/'),
            query.symbol,
        );
        let client = fmp_client().map_err(|e| Error::Provider(e.to_string()))?;
        let response = client
            .get(&url)
            .query(&[("apikey", api_key.as_str())])
            .send()
            .await
            .map_err(|e| Error::Provider(format!("fmp quote extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<unreadable>"));
            return Err(Error::Provider(format!(
                "fmp quote returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("fmp quote read body: {e}")))
    }

    fn transform_data(&self, query: &FmpQuoteQuery, raw: Bytes) -> Result<Vec<QuoteSnapshot>> {
        let entries: Vec<FmpQuoteRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp quote parse_json: {e}")))?;
        let rows = entries
            .into_iter()
            .map(|entry| QuoteSnapshot {
                symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                current_price: entry.price,
                change: entry.change,
                change_percent: entry.changes_percentage,
                prev_close: entry.previous_close,
                // FMP returns seconds; convert to milliseconds for the domain type.
                ts_ms: entry.timestamp * 1_000,
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// Shared helpers for the fundamentals cluster
// ---------------------------------------------------------------------------

/// Issue an authenticated GET against `url` with the given query pairs, mapping
/// transport / non-2xx responses to [`Error::Provider`] using `ctx` as the
/// error-text prefix. Returns the raw response body bytes.
async fn fmp_get(url: &str, params: &[(&str, String)], ctx: &str) -> Result<Bytes> {
    let api_key = api_key()?;
    let client = fmp_client().map_err(|e| Error::Provider(e.to_string()))?;
    let mut request = client.get(url).query(&[("apikey", api_key.as_str())]);
    for (key, value) in params {
        request = request.query(&[(*key, value.as_str())]);
    }
    let response = request
        .send()
        .await
        .map_err(|e| Error::Provider(format!("{ctx} extract_data: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| String::from("<unreadable>"));
        return Err(Error::Provider(format!("{ctx} returned {status}: {body}")));
    }
    response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("{ctx} read body: {e}")))
}

/// Convert an FMP camelCase / mixedCase JSON key to a normalized snake_case key
/// for the standardized `line_items` / `extra_*` bags. Pure ASCII transform:
/// inserts `_` before an uppercase letter that follows a lowercase letter or
/// digit, then lowercases.
fn to_snake_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    let mut prev_lower_or_digit = false;
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            if prev_lower_or_digit {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower_or_digit = false;
        } else {
            out.push(ch);
            prev_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    out
}

/// Read a string field from an FMP row object.
fn str_field(row: &Map<String, Value>, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

/// Read a numeric field (accepting JSON numbers or numeric strings) from a row.
fn num_field(row: &Map<String, Value>, key: &str) -> Option<f64> {
    match row.get(key) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Collect every numeric field of `row` not already consumed as a typed header
/// into a snake_case-keyed bag, skipping the keys in `skip`.
fn collect_numeric_bag(row: &Map<String, Value>, skip: &[&str]) -> BTreeMap<String, f64> {
    let mut bag = BTreeMap::new();
    for (key, value) in row {
        if skip.contains(&key.as_str()) {
            continue;
        }
        let parsed = match value {
            Value::Number(n) => n.as_f64(),
            _ => None,
        };
        if let Some(number) = parsed {
            bag.insert(to_snake_case(key), number);
        }
    }
    bag
}

/// Parse a top-level FMP array body into a vec of JSON objects, mapping decode
/// errors to [`Error::Provider`] with the `ctx` prefix.
fn parse_rows(raw: &Bytes, ctx: &str) -> Result<Vec<Map<String, Value>>> {
    serde_json::from_slice(raw).map_err(|e| Error::Provider(format!("{ctx} parse_json: {e}")))
}

// ---------------------------------------------------------------------------
// FmpHttpStatementFetcher — balance / income / cash (+ *-growth) → FinancialStatement
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP financial-statement fetcher.
    ///
    /// Normalizes the balance / income / cash-flow statements and their
    /// `*-statement-growth` siblings to the standardized
    /// [`FinancialStatement`] model: the period header is typed and every
    /// remaining numeric statement line lands in `line_items` keyed by a
    /// snake_case name.
    pub FmpHttpStatementFetcher,
    BASE_URL
);

/// Header keys lifted out of an FMP statement row into typed fields (never
/// swept into `line_items`).
const STATEMENT_HEADER_KEYS: &[&str] = &[
    "date",
    "symbol",
    "period",
    "calendarYear",
    "reportedCurrency",
    "fillingDate",
    "filingDate",
    "acceptedDate",
    "cik",
    "link",
    "finalLink",
];

#[async_trait]
impl Fetcher<FmpStatementQuery, FinancialStatement> for FmpHttpStatementFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "financial_statement";

    fn transform_query(params: Value) -> Result<FmpStatementQuery> {
        let symbol = symbol_param(&params)?;
        let statement = statement_param(&params)?;
        let growth = params
            .get("growth")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let period = crate::FmpPeriod::from_param(params.get("period").and_then(Value::as_str));
        let limit = limit_param(&params, 5)?;
        FmpStatementQuery::new(symbol, statement, growth, period, limit)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &FmpStatementQuery, _creds: &Credentials) -> Result<Bytes> {
        let segment = if query.growth {
            query.statement.as_growth_path_segment()
        } else {
            query.statement.as_path_segment()
        };
        let url = format!(
            "{}/{}/{}",
            self.base_url().trim_end_matches('/'),
            segment,
            query.symbol,
        );
        fmp_get(
            &url,
            &[
                ("period", query.period.as_param().to_string()),
                ("limit", query.limit.to_string()),
            ],
            "fmp statement",
        )
        .await
    }

    fn transform_data(
        &self,
        query: &FmpStatementQuery,
        raw: Bytes,
    ) -> Result<Vec<FinancialStatement>> {
        let kind = match query.statement {
            FmpStatement::Income => StatementKind::Income,
            FmpStatement::Balance => StatementKind::Balance,
            FmpStatement::Cashflow => StatementKind::Cash,
        };
        let rows = parse_rows(&raw, "fmp statement")?;
        let statements = rows
            .into_iter()
            .map(|row| {
                let fiscal_year = num_field(&row, "calendarYear").map(|y| y as i32);
                FinancialStatement {
                    symbol: str_field(&row, "symbol").unwrap_or_else(|| query.symbol.clone()),
                    statement: kind,
                    period: query.period.as_param().to_string(),
                    fiscal_year,
                    fiscal_period: str_field(&row, "period"),
                    date: str_field(&row, "date"),
                    filing_date: str_field(&row, "fillingDate")
                        .or_else(|| str_field(&row, "filingDate")),
                    currency: str_field(&row, "reportedCurrency"),
                    line_items: collect_numeric_bag(&row, STATEMENT_HEADER_KEYS),
                }
            })
            .collect();
        Ok(statements)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpKeyMetricsFetcher — /key-metrics → KeyMetrics
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP key-metrics fetcher (per-share & valuation metrics),
    /// normalized to [`KeyMetrics`].
    pub FmpHttpKeyMetricsFetcher,
    BASE_URL
);

/// Keys mapped onto typed [`KeyMetrics`] fields (excluded from `extra_metrics`).
const KEY_METRICS_TYPED_KEYS: &[&str] = &[
    "symbol",
    "date",
    "period",
    "calendarYear",
    "marketCap",
    "peRatio",
    "priceToSalesRatio",
    "pbRatio",
    "enterpriseValue",
    "enterpriseValueOverEBITDA",
    "netIncomePerShare",
    "revenuePerShare",
    "bookValuePerShare",
    "freeCashFlowPerShare",
    "dividendYield",
];

#[async_trait]
impl Fetcher<FmpFundamentalQuery, KeyMetrics> for FmpHttpKeyMetricsFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "key_metrics";

    fn transform_query(params: Value) -> Result<FmpFundamentalQuery> {
        fundamental_query(&params)
    }

    async fn extract_data(
        &self,
        query: &FmpFundamentalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/key-metrics/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(
            &url,
            &[
                ("period", query.period.as_param().to_string()),
                ("limit", query.limit.to_string()),
            ],
            "fmp key-metrics",
        )
        .await
    }

    fn transform_data(&self, query: &FmpFundamentalQuery, raw: Bytes) -> Result<Vec<KeyMetrics>> {
        let rows = parse_rows(&raw, "fmp key-metrics")?;
        let metrics = rows
            .into_iter()
            .map(|row| KeyMetrics {
                symbol: str_field(&row, "symbol").unwrap_or_else(|| query.symbol.clone()),
                period: str_field(&row, "period")
                    .or_else(|| Some(query.period.as_param().to_string())),
                date: str_field(&row, "date"),
                market_cap: num_field(&row, "marketCap"),
                pe_ratio: num_field(&row, "peRatio"),
                price_to_sales: num_field(&row, "priceToSalesRatio"),
                price_to_book: num_field(&row, "pbRatio"),
                enterprise_value: num_field(&row, "enterpriseValue"),
                ev_to_ebitda: num_field(&row, "enterpriseValueOverEBITDA"),
                earnings_per_share: num_field(&row, "netIncomePerShare"),
                revenue_per_share: num_field(&row, "revenuePerShare"),
                book_value_per_share: num_field(&row, "bookValuePerShare"),
                free_cash_flow_per_share: num_field(&row, "freeCashFlowPerShare"),
                dividend_yield: num_field(&row, "dividendYield"),
                extra_metrics: collect_numeric_bag(&row, KEY_METRICS_TYPED_KEYS),
            })
            .collect();
        Ok(metrics)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpRatiosFetcher — /ratios → Ratios
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP financial-ratios fetcher, normalized to [`Ratios`].
    pub FmpHttpRatiosFetcher,
    BASE_URL
);

/// Keys mapped onto typed [`Ratios`] fields (excluded from `extra_ratios`).
const RATIOS_TYPED_KEYS: &[&str] = &[
    "symbol",
    "date",
    "period",
    "calendarYear",
    "currentRatio",
    "quickRatio",
    "grossProfitMargin",
    "operatingProfitMargin",
    "netProfitMargin",
    "returnOnAssets",
    "returnOnEquity",
    "debtEquityRatio",
    "interestCoverage",
    "assetTurnover",
];

#[async_trait]
impl Fetcher<FmpFundamentalQuery, Ratios> for FmpHttpRatiosFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "ratios";

    fn transform_query(params: Value) -> Result<FmpFundamentalQuery> {
        fundamental_query(&params)
    }

    async fn extract_data(
        &self,
        query: &FmpFundamentalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/ratios/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(
            &url,
            &[
                ("period", query.period.as_param().to_string()),
                ("limit", query.limit.to_string()),
            ],
            "fmp ratios",
        )
        .await
    }

    fn transform_data(&self, query: &FmpFundamentalQuery, raw: Bytes) -> Result<Vec<Ratios>> {
        let rows = parse_rows(&raw, "fmp ratios")?;
        let ratios = rows
            .into_iter()
            .map(|row| Ratios {
                symbol: str_field(&row, "symbol").unwrap_or_else(|| query.symbol.clone()),
                period: str_field(&row, "period")
                    .or_else(|| Some(query.period.as_param().to_string())),
                date: str_field(&row, "date"),
                current_ratio: num_field(&row, "currentRatio"),
                quick_ratio: num_field(&row, "quickRatio"),
                gross_margin: num_field(&row, "grossProfitMargin"),
                operating_margin: num_field(&row, "operatingProfitMargin"),
                net_profit_margin: num_field(&row, "netProfitMargin"),
                return_on_assets: num_field(&row, "returnOnAssets"),
                return_on_equity: num_field(&row, "returnOnEquity"),
                debt_to_equity: num_field(&row, "debtEquityRatio"),
                interest_coverage: num_field(&row, "interestCoverage"),
                asset_turnover: num_field(&row, "assetTurnover"),
                extra_ratios: collect_numeric_bag(&row, RATIOS_TYPED_KEYS),
            })
            .collect();
        Ok(ratios)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpPeersFetcher — /stock_peers → Instrument
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP company-peers fetcher.
    ///
    /// Calls `/stock_peers?symbol=` and normalizes each comparable ticker to an
    /// [`Instrument`] (venue `"fmp"`, name set to the ticker since the peers
    /// endpoint returns symbols only).
    pub FmpHttpPeersFetcher,
    BASE_URL
);

/// Wire shape for the `/stock_peers` response (an array with a single entry).
#[derive(Deserialize)]
struct FmpPeersRaw {
    #[serde(rename = "peersList", default)]
    peers_list: Vec<String>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, Instrument> for FmpHttpPeersFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "peers";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        // The peers endpoint lives at the v4 root, not v3; derive it from the
        // configured base URL so mock servers still work.
        let url = format!("{}/stock_peers", peers_base(self.base_url()));
        fmp_get(&url, &[("symbol", query.symbol.clone())], "fmp peers").await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<Instrument>> {
        let entries: Vec<FmpPeersRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp peers parse_json: {e}")))?;
        let instruments = entries
            .into_iter()
            .flat_map(|entry| entry.peers_list)
            .filter(|symbol| !symbol.trim().is_empty())
            .map(|symbol| Instrument {
                name: symbol.clone(),
                symbol,
                venue: "fmp".to_string(),
            })
            .collect();
        let _ = query;
        Ok(instruments)
    }
}

/// Map the v3 base URL to the v4 root the peers endpoint lives under, leaving
/// non-standard (mock) base URLs untouched.
fn peers_base(base_url: &str) -> String {
    base_url
        .trim_end_matches('/')
        .strip_suffix("/v3")
        .map_or_else(
            || base_url.trim_end_matches('/').to_string(),
            |root| format!("{root}/v4"),
        )
}

// ---------------------------------------------------------------------------
// FmpHttpProfileFetcher — /profile → CompanyProfile
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP company-profile fetcher, normalized to [`CompanyProfile`].
    pub FmpHttpProfileFetcher,
    BASE_URL
);

/// Wire shape for a `/profile/{symbol}` entry.
#[derive(Deserialize)]
struct FmpProfileRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "companyName", default)]
    company_name: Option<String>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(rename = "exchangeShortName", default)]
    exchange_short_name: Option<String>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(rename = "mktCap", default)]
    mkt_cap: f64,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, CompanyProfile> for FmpHttpProfileFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "profile";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/profile/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(&url, &[], "fmp profile").await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<CompanyProfile>> {
        let entries: Vec<FmpProfileRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp profile parse_json: {e}")))?;
        let profiles = entries
            .into_iter()
            .map(|entry| CompanyProfile {
                ticker: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                name: entry.company_name.unwrap_or_default(),
                currency: entry.currency.unwrap_or_default(),
                exchange: entry
                    .exchange_short_name
                    .or(entry.exchange)
                    .unwrap_or_default(),
                logo_url: entry.image.unwrap_or_default(),
                // FMP reports absolute market cap; CompanyProfile stores millions.
                market_cap_millions: entry.mkt_cap / 1_000_000.0,
            })
            .collect();
        Ok(profiles)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpDividendsFetcher / FmpHttpSplitsFetcher — corporate actions
// ---------------------------------------------------------------------------

/// Wire shape for `/historical-price-full/stock_dividend|stock_split` bodies.
#[derive(Deserialize)]
struct FmpCorporateActionEnvelope {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    historical: Vec<FmpCorporateActionRaw>,
}

#[derive(Deserialize)]
struct FmpCorporateActionRaw {
    #[serde(default)]
    date: String,
    #[serde(default)]
    dividend: Option<f64>,
    #[serde(default)]
    numerator: Option<f64>,
    #[serde(default)]
    denominator: Option<f64>,
}

tdw_core::provider_fetcher_struct!(
    /// Production FMP historical-dividends fetcher, normalized to
    /// [`CorporateAction`] (`action_type = "dividend"`).
    pub FmpHttpDividendsFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FmpSymbolQuery, CorporateAction> for FmpHttpDividendsFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "dividends";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/historical-price-full/stock_dividend/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(&url, &[], "fmp dividends").await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<CorporateAction>> {
        let envelope: FmpCorporateActionEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp dividends parse_json: {e}")))?;
        let symbol = envelope.symbol.unwrap_or_else(|| query.symbol.clone());
        let actions = envelope
            .historical
            .into_iter()
            .map(|row| CorporateAction {
                symbol: symbol.clone(),
                ex_date: row.date,
                action_type: "dividend".to_string(),
                split_ratio: 0.0,
                cash_amount: row.dividend.unwrap_or(0.0),
                currency: String::new(),
            })
            .collect();
        Ok(actions)
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production FMP historical-splits fetcher, normalized to
    /// [`CorporateAction`] (`action_type = "split"`).
    pub FmpHttpSplitsFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FmpSymbolQuery, CorporateAction> for FmpHttpSplitsFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "splits";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/historical-price-full/stock_split/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(&url, &[], "fmp splits").await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<CorporateAction>> {
        let envelope: FmpCorporateActionEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp splits parse_json: {e}")))?;
        let symbol = envelope.symbol.unwrap_or_else(|| query.symbol.clone());
        let actions = envelope
            .historical
            .into_iter()
            .map(|row| {
                // FMP reports the split as numerator/denominator; the domain
                // `split_ratio` is the new-for-old factor (e.g. 4-for-1 = 4.0).
                let split_ratio = match (row.numerator, row.denominator) {
                    (Some(n), Some(d)) if d != 0.0 => n / d,
                    (Some(n), _) => n,
                    _ => 0.0,
                };
                CorporateAction {
                    symbol: symbol.clone(),
                    ex_date: row.date,
                    action_type: "split".to_string(),
                    split_ratio,
                    cash_amount: 0.0,
                    currency: String::new(),
                }
            })
            .collect();
        Ok(actions)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpEarningsFetcher — /historical/earning_calendar → Estimate (historical EPS)
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP historical-earnings (EPS) fetcher.
    ///
    /// Calls `/historical/earning_calendar/{symbol}` and normalizes each row to
    /// an [`Estimate`] with `kind = "historical_eps"`: `value` carries the
    /// reported (actual) EPS and `mean` the prior analyst estimate.
    pub FmpHttpEarningsFetcher,
    BASE_URL
);

/// Wire shape for a `/historical/earning_calendar/{symbol}` entry.
#[derive(Deserialize)]
struct FmpEarningsRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    eps: Option<f64>,
    #[serde(rename = "epsEstimated", default)]
    eps_estimated: Option<f64>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, Estimate> for FmpHttpEarningsFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "historical_eps";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/historical/earning_calendar/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(&url, &[], "fmp earnings").await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<Estimate>> {
        let entries: Vec<FmpEarningsRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp earnings parse_json: {e}")))?;
        let estimates = entries
            .into_iter()
            .map(|entry| Estimate {
                symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                kind: "historical_eps".to_string(),
                fiscal_period: None,
                date: entry.date,
                analyst: None,
                recommendation: None,
                value: entry.eps,
                low: None,
                high: None,
                mean: entry.eps_estimated,
                number_of_analysts: None,
                currency: None,
            })
            .collect();
        Ok(estimates)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpDiscoveryFetcher — /stock_market/{gainers,losers,actives} → QuoteSnapshot
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP market-movers fetcher.
    ///
    /// Calls `/stock_market/{gainers|losers|actives}` (the direction is selected
    /// per query) and normalizes each mover row to a [`QuoteSnapshot`]: `price`
    /// → `current_price`, the absolute `change`, and the percentage move. The
    /// movers feed carries no previous close or timestamp, so `prev_close` and
    /// `ts_ms` default to zero.
    pub FmpHttpDiscoveryFetcher,
    BASE_URL
);

/// Wire shape for a `/stock_market/{gainers|losers|actives}` row.
#[derive(Deserialize)]
struct FmpMoverRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    price: f64,
    #[serde(default)]
    change: f64,
    #[serde(rename = "changesPercentage", default)]
    changes_percentage: f64,
}

#[async_trait]
impl Fetcher<FmpDiscoveryQuery, QuoteSnapshot> for FmpHttpDiscoveryFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "discovery";

    fn transform_query(params: Value) -> Result<FmpDiscoveryQuery> {
        let direction =
            FmpDiscoveryDirection::from_param(params.get("direction").and_then(Value::as_str));
        Ok(FmpDiscoveryQuery::new(direction))
    }

    async fn extract_data(&self, query: &FmpDiscoveryQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/stock_market/{}",
            self.base_url().trim_end_matches('/'),
            query.direction.as_path_segment(),
        );
        fmp_get(&url, &[], "fmp discovery").await
    }

    fn transform_data(&self, query: &FmpDiscoveryQuery, raw: Bytes) -> Result<Vec<QuoteSnapshot>> {
        let entries: Vec<FmpMoverRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp discovery parse_json: {e}")))?;
        let snapshots = entries
            .into_iter()
            .filter_map(|entry| {
                entry.symbol.map(|symbol| QuoteSnapshot {
                    symbol,
                    current_price: entry.price,
                    change: entry.change,
                    change_percent: entry.changes_percentage,
                    prev_close: 0.0,
                    ts_ms: 0,
                })
            })
            .collect();
        let _ = query;
        Ok(snapshots)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpScreenerFetcher — /stock-screener → ScreenerRow
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP equity-screener fetcher, normalized to [`ScreenerRow`].
    ///
    /// Calls `/stock-screener` with the caller's optional filters
    /// (`marketCapMoreThan`, `sector`, `industry`, `exchange`, `limit`, …) and
    /// maps each result company to a [`ScreenerRow`].
    pub FmpHttpScreenerFetcher,
    BASE_URL
);

/// Wire shape for a `/stock-screener` result row.
#[derive(Deserialize)]
struct FmpScreenerRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "companyName", default)]
    company_name: Option<String>,
    #[serde(rename = "marketCap", default)]
    market_cap: Option<f64>,
    #[serde(default)]
    sector: Option<String>,
    #[serde(default)]
    industry: Option<String>,
    #[serde(default)]
    beta: Option<f64>,
    #[serde(default)]
    price: Option<f64>,
    #[serde(rename = "lastAnnualDividend", default)]
    last_annual_dividend: Option<f64>,
    #[serde(default)]
    volume: Option<f64>,
    #[serde(default)]
    exchange: Option<String>,
    #[serde(rename = "exchangeShortName", default)]
    exchange_short_name: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(rename = "isEtf", default)]
    is_etf: Option<bool>,
    #[serde(rename = "isActivelyTrading", default)]
    is_actively_trading: Option<bool>,
}

#[async_trait]
impl Fetcher<FmpScreenerQuery, ScreenerRow> for FmpHttpScreenerFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "screener";

    fn transform_query(params: Value) -> Result<FmpScreenerQuery> {
        let string_param = |key: &str| {
            params
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
        };
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|e| Error::InvalidQuery(format!("fmp screener limit too large: {e}")))?;
        Ok(FmpScreenerQuery {
            market_cap_more_than: params.get("market_cap_more_than").and_then(Value::as_f64),
            market_cap_lower_than: params.get("market_cap_lower_than").and_then(Value::as_f64),
            sector: string_param("sector"),
            industry: string_param("industry"),
            exchange: string_param("exchange"),
            limit,
        })
    }

    async fn extract_data(&self, query: &FmpScreenerQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!("{}/stock-screener", self.base_url().trim_end_matches('/'));
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(value) = query.market_cap_more_than {
            params.push(("marketCapMoreThan", value.to_string()));
        }
        if let Some(value) = query.market_cap_lower_than {
            params.push(("marketCapLowerThan", value.to_string()));
        }
        if let Some(sector) = &query.sector {
            params.push(("sector", sector.clone()));
        }
        if let Some(industry) = &query.industry {
            params.push(("industry", industry.clone()));
        }
        if let Some(exchange) = &query.exchange {
            params.push(("exchange", exchange.clone()));
        }
        if let Some(limit) = query.limit {
            params.push(("limit", limit.to_string()));
        }
        fmp_get(&url, &params, "fmp screener").await
    }

    fn transform_data(&self, query: &FmpScreenerQuery, raw: Bytes) -> Result<Vec<ScreenerRow>> {
        let entries: Vec<FmpScreenerRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp screener parse_json: {e}")))?;
        let rows = entries
            .into_iter()
            .filter_map(|entry| {
                entry.symbol.map(|symbol| ScreenerRow {
                    symbol,
                    company_name: entry.company_name,
                    market_cap: entry.market_cap,
                    sector: entry.sector,
                    industry: entry.industry,
                    beta: entry.beta,
                    price: entry.price,
                    last_annual_dividend: entry.last_annual_dividend,
                    volume: entry.volume,
                    exchange: entry.exchange,
                    exchange_short_name: entry.exchange_short_name,
                    country: entry.country,
                    is_etf: entry.is_etf,
                    is_actively_trading: entry.is_actively_trading,
                })
            })
            .collect();
        let _ = query;
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpPriceTargetFetcher — /v4/price-target-consensus → Estimate (price_target)
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP price-target-consensus fetcher.
    ///
    /// Calls `/v4/price-target-consensus?symbol=` and normalizes the consensus
    /// row to a single [`Estimate`] with `kind = "price_target"`: `value` and
    /// `mean` carry the consensus target, with `high`/`low` from the analyst
    /// range.
    pub FmpHttpPriceTargetFetcher,
    BASE_URL
);

/// Wire shape for a `/v4/price-target-consensus` entry.
#[derive(Deserialize)]
struct FmpPriceTargetRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "targetHigh", default)]
    target_high: Option<f64>,
    #[serde(rename = "targetLow", default)]
    target_low: Option<f64>,
    #[serde(rename = "targetConsensus", default)]
    target_consensus: Option<f64>,
    #[serde(rename = "targetMedian", default)]
    target_median: Option<f64>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, Estimate> for FmpHttpPriceTargetFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "price_target";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        // The price-target-consensus endpoint lives at the v4 root, not v3;
        // derive it from the configured base URL so mock servers still work.
        let url = format!("{}/price-target-consensus", peers_base(self.base_url()));
        fmp_get(
            &url,
            &[("symbol", query.symbol.clone())],
            "fmp price_target",
        )
        .await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<Estimate>> {
        let entries: Vec<FmpPriceTargetRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp price_target parse_json: {e}")))?;
        let estimates = entries
            .into_iter()
            .map(|entry| {
                let consensus = entry.target_consensus.or(entry.target_median);
                Estimate {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    kind: "price_target".to_string(),
                    fiscal_period: None,
                    date: None,
                    analyst: None,
                    recommendation: None,
                    value: consensus,
                    low: entry.target_low,
                    high: entry.target_high,
                    mean: consensus,
                    number_of_analysts: None,
                    currency: None,
                }
            })
            .collect();
        Ok(estimates)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpAnalystEstimatesFetcher — /analyst-estimates/{symbol} → Estimate (forward_*)
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP forward analyst-estimates fetcher.
    ///
    /// Calls `/analyst-estimates/{symbol}?period=annual` and emits one
    /// [`Estimate`] row per forward metric per period: `forward_eps` (from the
    /// `estimatedEps*` triplet), `forward_sales` (`estimatedRevenue*`), and
    /// `forward_ebitda` (`estimatedEbitda*`). Each row carries `fiscal_period`
    /// = the period date and `value`/`low`/`high`/`mean` from the
    /// `*Avg`/`*Low`/`*High` fields; a metric whose `*Avg` is absent is skipped.
    pub FmpHttpAnalystEstimatesFetcher,
    BASE_URL
);

/// Wire shape for an `/analyst-estimates/{symbol}` period entry.
#[derive(Deserialize)]
struct FmpAnalystEstimateRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "estimatedRevenueLow", default)]
    estimated_revenue_low: Option<f64>,
    #[serde(rename = "estimatedRevenueHigh", default)]
    estimated_revenue_high: Option<f64>,
    #[serde(rename = "estimatedRevenueAvg", default)]
    estimated_revenue_avg: Option<f64>,
    #[serde(rename = "estimatedEpsLow", default)]
    estimated_eps_low: Option<f64>,
    #[serde(rename = "estimatedEpsHigh", default)]
    estimated_eps_high: Option<f64>,
    #[serde(rename = "estimatedEpsAvg", default)]
    estimated_eps_avg: Option<f64>,
    #[serde(rename = "estimatedEbitdaLow", default)]
    estimated_ebitda_low: Option<f64>,
    #[serde(rename = "estimatedEbitdaHigh", default)]
    estimated_ebitda_high: Option<f64>,
    #[serde(rename = "estimatedEbitdaAvg", default)]
    estimated_ebitda_avg: Option<f64>,
    #[serde(rename = "numberAnalystEstimatedRevenue", default)]
    number_analyst_revenue: Option<u32>,
    #[serde(rename = "numberAnalystsEstimatedEps", default)]
    number_analyst_eps: Option<u32>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, Estimate> for FmpHttpAnalystEstimatesFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "analyst_estimates";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/analyst-estimates/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(
            &url,
            &[("period", "annual".to_string())],
            "fmp analyst_estimates",
        )
        .await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<Estimate>> {
        let entries: Vec<FmpAnalystEstimateRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp analyst_estimates parse_json: {e}")))?;
        let mut estimates = Vec::new();
        for entry in entries {
            let symbol = entry.symbol.unwrap_or_else(|| query.symbol.clone());
            let period = entry.date;
            let mut push = |kind: &str,
                            avg: Option<f64>,
                            low: Option<f64>,
                            high: Option<f64>,
                            analysts: Option<u32>| {
                if let Some(value) = avg {
                    estimates.push(Estimate {
                        symbol: symbol.clone(),
                        kind: kind.to_string(),
                        fiscal_period: period.clone(),
                        date: None,
                        analyst: None,
                        recommendation: None,
                        value: Some(value),
                        low,
                        high,
                        mean: Some(value),
                        number_of_analysts: analysts,
                        currency: None,
                    });
                }
            };
            push(
                "forward_eps",
                entry.estimated_eps_avg,
                entry.estimated_eps_low,
                entry.estimated_eps_high,
                entry.number_analyst_eps,
            );
            push(
                "forward_sales",
                entry.estimated_revenue_avg,
                entry.estimated_revenue_low,
                entry.estimated_revenue_high,
                entry.number_analyst_revenue,
            );
            push(
                "forward_ebitda",
                entry.estimated_ebitda_avg,
                entry.estimated_ebitda_low,
                entry.estimated_ebitda_high,
                None,
            );
        }
        Ok(estimates)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpKeyExecutivesFetcher — /key-executives/{symbol} → KeyExecutive
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP key-executives fetcher, normalized to [`KeyExecutive`].
    ///
    /// Calls `/key-executives/{symbol}` and maps each officer entry to a
    /// [`KeyExecutive`] (the management team).
    pub FmpHttpKeyExecutivesFetcher,
    BASE_URL
);

/// Wire shape for a `/key-executives/{symbol}` entry.
#[derive(Deserialize)]
struct FmpKeyExecutiveRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    pay: Option<f64>,
    #[serde(rename = "currencyPay", default)]
    currency_pay: Option<String>,
    #[serde(default)]
    gender: Option<String>,
    #[serde(rename = "yearBorn", default)]
    year_born: Option<i32>,
    #[serde(rename = "titleSince", default)]
    title_since: Option<i32>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, KeyExecutive> for FmpHttpKeyExecutivesFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "key_executives";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/key-executives/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(&url, &[], "fmp key_executives").await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<KeyExecutive>> {
        let entries: Vec<FmpKeyExecutiveRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp key_executives parse_json: {e}")))?;
        let executives = entries
            .into_iter()
            .filter_map(|entry| {
                // Defensively drop entries with an empty name so an empty
                // provider string can't fail domain validation (mirrors the
                // transcript fetcher's empty-content guard).
                let name = entry.name.filter(|n| !n.is_empty())?;
                Some(KeyExecutive {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    name,
                    title: entry.title,
                    pay: entry.pay,
                    currency: entry.currency_pay,
                    gender: entry.gender,
                    year_born: entry.year_born,
                    title_since: entry.title_since,
                })
            })
            .collect();
        Ok(executives)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpExecutiveCompensationFetcher — /v4/governance/executive_compensation
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP executive-compensation fetcher, normalized to
    /// [`ExecutiveCompensation`].
    ///
    /// Calls `/v4/governance/executive_compensation?symbol=` and maps each
    /// disclosed officer/year row to an [`ExecutiveCompensation`].
    pub FmpHttpExecutiveCompensationFetcher,
    BASE_URL
);

/// Wire shape for a `/v4/governance/executive_compensation` entry.
#[derive(Deserialize)]
struct FmpExecutiveCompensationRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "nameAndPosition", default)]
    name_and_position: Option<String>,
    #[serde(default)]
    year: Option<i32>,
    #[serde(rename = "acceptedDate", default)]
    accepted_date: Option<String>,
    #[serde(default)]
    salary: Option<f64>,
    #[serde(default)]
    bonus: Option<f64>,
    #[serde(rename = "stock_award", default)]
    stock_award: Option<f64>,
    #[serde(rename = "option_award", default)]
    option_award: Option<f64>,
    #[serde(rename = "incentive_plan_compensation", default)]
    incentive_plan_compensation: Option<f64>,
    #[serde(rename = "all_other_compensation", default)]
    all_other_compensation: Option<f64>,
    #[serde(default)]
    total: Option<f64>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, ExecutiveCompensation> for FmpHttpExecutiveCompensationFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "executive_compensation";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/governance/executive_compensation",
            peers_base(self.base_url()),
        );
        fmp_get(
            &url,
            &[("symbol", query.symbol.clone())],
            "fmp executive_compensation",
        )
        .await
    }

    fn transform_data(
        &self,
        query: &FmpSymbolQuery,
        raw: Bytes,
    ) -> Result<Vec<ExecutiveCompensation>> {
        let entries: Vec<FmpExecutiveCompensationRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp executive_compensation parse_json: {e}")))?;
        let rows = entries
            .into_iter()
            .filter_map(|entry| {
                // Defensively drop entries with an empty name so an empty
                // provider string can't fail domain validation (mirrors the
                // transcript fetcher's empty-content guard).
                let name_and_position = entry.name_and_position.filter(|n| !n.is_empty())?;
                Some(ExecutiveCompensation {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    name_and_position,
                    fiscal_year: entry.year,
                    filing_date: entry.accepted_date,
                    salary: entry.salary,
                    bonus: entry.bonus,
                    stock_award: entry.stock_award,
                    option_award: entry.option_award,
                    incentive_plan_compensation: entry.incentive_plan_compensation,
                    all_other_compensation: entry.all_other_compensation,
                    total: entry.total,
                    currency: None,
                })
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpRevenueSegmentFetcher — /v4/revenue-{product,geographic}-segmentation
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP revenue-segmentation fetcher, normalized to
    /// [`RevenueSegment`].
    ///
    /// Calls `/v4/revenue-product-segmentation` or
    /// `/v4/revenue-geographic-segmentation` (selected by the query's
    /// [`FmpSegmentKind`]) and flattens FMP's `{date: {segment: revenue}}` map
    /// into one [`RevenueSegment`] row per (period, segment).
    pub FmpHttpRevenueSegmentFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FmpRevenueSegmentQuery, RevenueSegment> for FmpHttpRevenueSegmentFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "revenue_segment";

    fn transform_query(params: Value) -> Result<FmpRevenueSegmentQuery> {
        let symbol = symbol_param(&params)?;
        let kind = FmpSegmentKind::from_param(
            params
                .get("structure")
                .or_else(|| params.get("segment_kind"))
                .and_then(Value::as_str),
        );
        let period = crate::FmpPeriod::from_param(params.get("period").and_then(Value::as_str));
        FmpRevenueSegmentQuery::new(symbol, kind, period)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpRevenueSegmentQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/{}",
            peers_base(self.base_url()),
            query.kind.as_path_segment(),
        );
        fmp_get(
            &url,
            &[
                ("symbol", query.symbol.clone()),
                ("period", query.period.as_param().to_string()),
                ("structure", "flat".to_string()),
            ],
            "fmp revenue_segment",
        )
        .await
    }

    fn transform_data(
        &self,
        query: &FmpRevenueSegmentQuery,
        raw: Bytes,
    ) -> Result<Vec<RevenueSegment>> {
        // FMP returns an array of single-key objects: `[{ "2024-09-28": { "iPhone": 201.. } }]`.
        let entries: Vec<Map<String, Value>> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp revenue_segment parse_json: {e}")))?;
        let mut rows = Vec::new();
        for entry in entries {
            for (date, segments) in entry {
                let Some(segment_map) = segments.as_object() else {
                    continue;
                };
                for (segment, value) in segment_map {
                    rows.push(RevenueSegment {
                        symbol: query.symbol.clone(),
                        kind: query.kind.as_kind().to_string(),
                        date: date.clone(),
                        segment: segment.clone(),
                        revenue: value.as_f64(),
                    });
                }
            }
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpTranscriptFetcher — /earning_call_transcript/{symbol} → EarningsTranscript
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP earnings-call-transcript fetcher, normalized to
    /// [`EarningsTranscript`].
    ///
    /// Calls `/earning_call_transcript/{symbol}?year=&quarter=` and maps each
    /// returned transcript to an [`EarningsTranscript`].
    pub FmpHttpTranscriptFetcher,
    BASE_URL
);

/// Wire shape for an `/earning_call_transcript/{symbol}` entry.
#[derive(Deserialize)]
struct FmpTranscriptRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    year: Option<i32>,
    #[serde(default)]
    quarter: Option<i32>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

#[async_trait]
impl Fetcher<FmpTranscriptQuery, EarningsTranscript> for FmpHttpTranscriptFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "transcript";

    fn transform_query(params: Value) -> Result<FmpTranscriptQuery> {
        let symbol = symbol_param(&params)?;
        let year = u32::try_from(params.get("year").and_then(Value::as_u64).unwrap_or(0))
            .map_err(|e| Error::InvalidQuery(format!("fmp transcript year too large: {e}")))?;
        let quarter = u32::try_from(params.get("quarter").and_then(Value::as_u64).unwrap_or(0))
            .map_err(|e| Error::InvalidQuery(format!("fmp transcript quarter too large: {e}")))?;
        FmpTranscriptQuery::new(symbol, year, quarter)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpTranscriptQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/earning_call_transcript/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(
            &url,
            &[
                ("year", query.year.to_string()),
                ("quarter", query.quarter.to_string()),
            ],
            "fmp transcript",
        )
        .await
    }

    fn transform_data(
        &self,
        query: &FmpTranscriptQuery,
        raw: Bytes,
    ) -> Result<Vec<EarningsTranscript>> {
        let entries: Vec<FmpTranscriptRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp transcript parse_json: {e}")))?;
        let transcripts = entries
            .into_iter()
            .filter_map(|entry| {
                let content = entry.content.filter(|c| !c.is_empty())?;
                Some(EarningsTranscript {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    year: entry.year.or_else(|| i32::try_from(query.year).ok()),
                    quarter: entry.quarter.or_else(|| i32::try_from(query.quarter).ok()),
                    date: entry.date,
                    content,
                })
            })
            .collect();
        Ok(transcripts)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpEsgScoreFetcher — /v4/esg-environmental-social-governance-data → EsgScore
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP ESG-score fetcher, normalized to [`EsgScore`].
    ///
    /// Calls `/v4/esg-environmental-social-governance-data?symbol=` and maps each
    /// disclosure to an [`EsgScore`].
    pub FmpHttpEsgScoreFetcher,
    BASE_URL
);

/// Wire shape for a `/v4/esg-environmental-social-governance-data` entry.
#[derive(Deserialize)]
struct FmpEsgScoreRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "companyName", default)]
    company_name: Option<String>,
    #[serde(rename = "environmentalScore", default)]
    environmental_score: Option<f64>,
    #[serde(rename = "socialScore", default)]
    social_score: Option<f64>,
    #[serde(rename = "governanceScore", default)]
    governance_score: Option<f64>,
    #[serde(rename = "ESGScore", default)]
    esg_score: Option<f64>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, EsgScore> for FmpHttpEsgScoreFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "esg_score";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/esg-environmental-social-governance-data",
            peers_base(self.base_url()),
        );
        fmp_get(&url, &[("symbol", query.symbol.clone())], "fmp esg_score").await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<EsgScore>> {
        let entries: Vec<FmpEsgScoreRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp esg_score parse_json: {e}")))?;
        let scores = entries
            .into_iter()
            .filter_map(|entry| {
                let date = entry.date?;
                Some(EsgScore {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    date,
                    company_name: entry.company_name,
                    environmental_score: entry.environmental_score,
                    social_score: entry.social_score,
                    governance_score: entry.governance_score,
                    esg_score: entry.esg_score,
                })
            })
            .collect();
        Ok(scores)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpEmployeeCountFetcher — /v4/historical/employee_count → EmployeeCount
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP historical-employee-count fetcher, normalized to
    /// [`EmployeeCount`].
    ///
    /// Calls `/v4/historical/employee_count?symbol=` and maps each filing to an
    /// [`EmployeeCount`].
    pub FmpHttpEmployeeCountFetcher,
    BASE_URL
);

/// Wire shape for a `/v4/historical/employee_count` entry.
#[derive(Deserialize)]
struct FmpEmployeeCountRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "periodOfReport", default)]
    period_of_report: Option<String>,
    #[serde(rename = "filingDate", default)]
    filing_date: Option<String>,
    #[serde(rename = "employeeCount", default)]
    employee_count: Option<i64>,
    #[serde(default)]
    source: Option<String>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, EmployeeCount> for FmpHttpEmployeeCountFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "employee_count";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!("{}/historical/employee_count", peers_base(self.base_url()));
        fmp_get(
            &url,
            &[("symbol", query.symbol.clone())],
            "fmp employee_count",
        )
        .await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<EmployeeCount>> {
        let entries: Vec<FmpEmployeeCountRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp employee_count parse_json: {e}")))?;
        let counts = entries
            .into_iter()
            .filter_map(|entry| {
                let period_of_report = entry.period_of_report?;
                Some(EmployeeCount {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    period_of_report,
                    filing_date: entry.filing_date,
                    employee_count: entry.employee_count,
                    source: entry.source,
                })
            })
            .collect();
        Ok(counts)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpFilingsFetcher — /sec_filings/{symbol} → CompanyFiling
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP SEC-filings-index fetcher, normalized to [`CompanyFiling`].
    ///
    /// Calls `/sec_filings/{symbol}?limit=` and maps each filing-index entry to a
    /// [`CompanyFiling`].
    pub FmpHttpFilingsFetcher,
    BASE_URL
);

/// Wire shape for a `/sec_filings/{symbol}` entry.
#[derive(Deserialize)]
struct FmpFilingRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "type", default)]
    form_type: Option<String>,
    #[serde(rename = "fillingDate", default)]
    filling_date: Option<String>,
    #[serde(rename = "filingDate", default)]
    filing_date: Option<String>,
    #[serde(rename = "acceptedDate", default)]
    accepted_date: Option<String>,
    #[serde(default)]
    cik: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(rename = "finalLink", default)]
    final_link: Option<String>,
}

#[async_trait]
impl Fetcher<FmpSymbolLimitQuery, CompanyFiling> for FmpHttpFilingsFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "filings";

    fn transform_query(params: Value) -> Result<FmpSymbolLimitQuery> {
        let symbol = symbol_param(&params)?;
        let limit = limit_param(&params, 100)?;
        FmpSymbolLimitQuery::new(symbol, limit).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpSymbolLimitQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/sec_filings/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(&url, &[("limit", query.limit.to_string())], "fmp filings").await
    }

    fn transform_data(
        &self,
        query: &FmpSymbolLimitQuery,
        raw: Bytes,
    ) -> Result<Vec<CompanyFiling>> {
        let entries: Vec<FmpFilingRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp filings parse_json: {e}")))?;
        let filings = entries
            .into_iter()
            .filter_map(|entry| {
                let form_type = entry.form_type?;
                Some(CompanyFiling {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    form_type,
                    filing_date: entry.filling_date.or(entry.filing_date),
                    accepted_date: entry.accepted_date,
                    cik: entry.cik,
                    link: entry.link,
                    final_link: entry.final_link,
                })
            })
            .collect();
        Ok(filings)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpSearchFetcher — /search → Instrument (equity/search)
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP ticker-search fetcher, normalized to [`Instrument`].
    ///
    /// Calls `/search?query=&limit=` and maps each match to an [`Instrument`]
    /// (the search hit's symbol, company name, and exchange).
    pub FmpHttpSearchFetcher,
    BASE_URL
);

/// Wire shape for a `/search` result entry.
#[derive(Deserialize)]
struct FmpSearchRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "exchangeShortName", default)]
    exchange_short_name: Option<String>,
    #[serde(rename = "stockExchange", default)]
    stock_exchange: Option<String>,
}

#[async_trait]
impl Fetcher<FmpSearchQuery, Instrument> for FmpHttpSearchFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "search";

    fn transform_query(params: Value) -> Result<FmpSearchQuery> {
        let query = params
            .get("query")
            .or_else(|| params.get("q"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("fmp search query must be a string".to_string()))?;
        let limit = limit_param(&params, 10)?;
        FmpSearchQuery::new(query, limit).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &FmpSearchQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!("{}/search", self.base_url().trim_end_matches('/'));
        fmp_get(
            &url,
            &[
                ("query", query.query.clone()),
                ("limit", query.limit.to_string()),
            ],
            "fmp search",
        )
        .await
    }

    fn transform_data(&self, query: &FmpSearchQuery, raw: Bytes) -> Result<Vec<Instrument>> {
        let entries: Vec<FmpSearchRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp search parse_json: {e}")))?;
        let instruments = entries
            .into_iter()
            .filter_map(|entry| {
                let symbol = entry.symbol.filter(|s| !s.trim().is_empty())?;
                Some(Instrument {
                    name: entry.name.unwrap_or_else(|| symbol.clone()),
                    symbol,
                    venue: entry
                        .exchange_short_name
                        .or(entry.stock_exchange)
                        .unwrap_or_else(|| "fmp".to_string()),
                })
            })
            .collect();
        let _ = query;
        Ok(instruments)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpHistoricalMarketCapFetcher — /historical-market-capitalization
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP historical-market-capitalization fetcher, normalized to
    /// [`HistoricalMarketCap`].
    ///
    /// Calls `/historical-market-capitalization/{symbol}?limit=` and maps each
    /// (date, market-cap) point to a [`HistoricalMarketCap`].
    pub FmpHttpHistoricalMarketCapFetcher,
    BASE_URL
);

/// Wire shape for a `/historical-market-capitalization/{symbol}` entry.
#[derive(Deserialize)]
struct FmpMarketCapRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "marketCap", default)]
    market_cap: Option<f64>,
}

#[async_trait]
impl Fetcher<FmpSymbolLimitQuery, HistoricalMarketCap> for FmpHttpHistoricalMarketCapFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "historical_market_cap";

    fn transform_query(params: Value) -> Result<FmpSymbolLimitQuery> {
        let symbol = symbol_param(&params)?;
        let limit = limit_param(&params, 100)?;
        FmpSymbolLimitQuery::new(symbol, limit).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpSymbolLimitQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/historical-market-capitalization/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(
            &url,
            &[("limit", query.limit.to_string())],
            "fmp historical_market_cap",
        )
        .await
    }

    fn transform_data(
        &self,
        query: &FmpSymbolLimitQuery,
        raw: Bytes,
    ) -> Result<Vec<HistoricalMarketCap>> {
        let entries: Vec<FmpMarketCapRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp historical_market_cap parse_json: {e}")))?;
        let rows = entries
            .into_iter()
            .filter_map(|entry| {
                let date = entry.date?;
                Some(HistoricalMarketCap {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    date,
                    market_cap: entry.market_cap,
                })
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpSplitCalendarFetcher — /stock_split_calendar → CalendarEvent
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP stock-split-calendar fetcher, normalized to
    /// [`CalendarEvent`] (`kind = "split"`).
    ///
    /// Calls `/stock_split_calendar?from=&to=` and maps each upcoming split to a
    /// [`CalendarEvent`].
    pub FmpHttpSplitCalendarFetcher,
    BASE_URL
);

/// Wire shape for a `/stock_split_calendar` entry.
#[derive(Deserialize)]
struct FmpSplitCalendarRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    numerator: Option<f64>,
    #[serde(default)]
    denominator: Option<f64>,
}

#[async_trait]
impl Fetcher<FmpCalendarRangeQuery, CalendarEvent> for FmpHttpSplitCalendarFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "split_calendar";

    fn transform_query(params: Value) -> Result<FmpCalendarRangeQuery> {
        let from = params.get("from").and_then(Value::as_str);
        let to = params.get("to").and_then(Value::as_str);
        FmpCalendarRangeQuery::new(from, to).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &FmpCalendarRangeQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/stock_split_calendar",
            self.base_url().trim_end_matches('/')
        );
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(from) = &query.from {
            params.push(("from", from.clone()));
        }
        if let Some(to) = &query.to {
            params.push(("to", to.clone()));
        }
        fmp_get(&url, &params, "fmp split_calendar").await
    }

    fn transform_data(
        &self,
        query: &FmpCalendarRangeQuery,
        raw: Bytes,
    ) -> Result<Vec<CalendarEvent>> {
        let entries: Vec<FmpSplitCalendarRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp split_calendar parse_json: {e}")))?;
        let rows = entries
            .into_iter()
            .filter_map(|entry| {
                let symbol = entry.symbol.filter(|s| !s.trim().is_empty())?;
                // The split factor (new-for-old) carried in the price field.
                let ratio = match (entry.numerator, entry.denominator) {
                    (Some(n), Some(d)) if d != 0.0 => Some(n / d),
                    (Some(n), _) => Some(n),
                    _ => None,
                };
                Some(CalendarEvent {
                    kind: "split".to_string(),
                    symbol,
                    name: entry.label,
                    date: entry.date,
                    dividend: None,
                    payment_date: None,
                    record_date: None,
                    eps_estimate: None,
                    fiscal_period: None,
                    price: ratio,
                    shares: None,
                    exchange: None,
                })
            })
            .collect();
        let _ = query;
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpLatestFilingsFetcher — /rss_feed → CompanyFiling (equity/discovery/filings)
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP latest-SEC-filings-feed fetcher, normalized to
    /// [`CompanyFiling`].
    ///
    /// Calls `/rss_feed?limit=` (the cross-issuer feed of the most-recent SEC
    /// filings reported to EDGAR) and maps each feed item to a [`CompanyFiling`].
    pub FmpHttpLatestFilingsFetcher,
    BASE_URL
);

/// Wire shape for a `/rss_feed` entry.
#[derive(Deserialize)]
struct FmpRssFilingRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "type", default)]
    form_type: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "acceptedDate", default)]
    accepted_date: Option<String>,
    #[serde(default)]
    cik: Option<String>,
    #[serde(default)]
    link: Option<String>,
    #[serde(rename = "finalLink", default)]
    final_link: Option<String>,
}

#[async_trait]
impl Fetcher<FmpLimitQuery, CompanyFiling> for FmpHttpLatestFilingsFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "latest_filings";

    fn transform_query(params: Value) -> Result<FmpLimitQuery> {
        let limit = limit_param(&params, 100)?;
        FmpLimitQuery::new(limit).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &FmpLimitQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!("{}/rss_feed", self.base_url().trim_end_matches('/'));
        fmp_get(
            &url,
            &[("limit", query.limit.to_string())],
            "fmp latest_filings",
        )
        .await
    }

    fn transform_data(&self, query: &FmpLimitQuery, raw: Bytes) -> Result<Vec<CompanyFiling>> {
        let entries: Vec<FmpRssFilingRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp latest_filings parse_json: {e}")))?;
        let filings = entries
            .into_iter()
            .filter_map(|entry| {
                let symbol = entry.symbol.filter(|s| !s.trim().is_empty())?;
                let form_type = entry.form_type.filter(|t| !t.trim().is_empty())?;
                Some(CompanyFiling {
                    symbol,
                    form_type,
                    filing_date: entry.date,
                    accepted_date: entry.accepted_date,
                    cik: entry.cik,
                    link: entry.link,
                    final_link: entry.final_link,
                })
            })
            .collect();
        let _ = query;
        Ok(filings)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpInsiderTradingFetcher — /v4/insider-trading → OwnershipRecord
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP insider-trading fetcher, normalized to [`OwnershipRecord`]
    /// (`kind = "insider"`).
    ///
    /// Calls `/v4/insider-trading?symbol=` and maps each reported transaction to
    /// an [`OwnershipRecord`].
    pub FmpHttpInsiderTradingFetcher,
    BASE_URL
);

/// Wire shape for a `/v4/insider-trading` entry.
#[derive(Deserialize)]
struct FmpInsiderTradeRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "reportingName", default)]
    reporting_name: Option<String>,
    #[serde(rename = "typeOfOwner", default)]
    type_of_owner: Option<String>,
    #[serde(rename = "transactionDate", default)]
    transaction_date: Option<String>,
    #[serde(rename = "transactionType", default)]
    transaction_type: Option<String>,
    #[serde(rename = "securitiesTransacted", default)]
    securities_transacted: Option<f64>,
    #[serde(default)]
    price: Option<f64>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, OwnershipRecord> for FmpHttpInsiderTradingFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "insider_trading";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        // The insider-trading endpoint lives at the v4 root, not v3.
        let url = format!("{}/insider-trading", peers_base(self.base_url()));
        fmp_get(
            &url,
            &[("symbol", query.symbol.clone())],
            "fmp insider_trading",
        )
        .await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<OwnershipRecord>> {
        let entries: Vec<FmpInsiderTradeRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp insider_trading parse_json: {e}")))?;
        let rows = entries
            .into_iter()
            .map(|entry| {
                let shares = entry.securities_transacted;
                let value = match (shares, entry.price) {
                    (Some(s), Some(p)) => Some(s * p),
                    _ => None,
                };
                OwnershipRecord {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    kind: "insider".to_string(),
                    holder: entry.reporting_name,
                    relationship: entry.type_of_owner,
                    date: entry.transaction_date,
                    transaction_type: entry.transaction_type,
                    shares,
                    value,
                    percentage: None,
                }
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpInstitutionalOwnershipFetcher — /institutional-holder → OwnershipRecord
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP institutional-ownership fetcher, normalized to
    /// [`OwnershipRecord`] (`kind = "institutional"`).
    ///
    /// Calls `/institutional-holder/{symbol}` and maps each institutional holder
    /// to an [`OwnershipRecord`].
    pub FmpHttpInstitutionalOwnershipFetcher,
    BASE_URL
);

/// Wire shape for an `/institutional-holder/{symbol}` entry.
#[derive(Deserialize)]
struct FmpInstitutionalHolderRaw {
    #[serde(default)]
    holder: Option<String>,
    #[serde(default)]
    shares: Option<f64>,
    #[serde(rename = "dateReported", default)]
    date_reported: Option<String>,
    #[serde(default)]
    change: Option<f64>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, OwnershipRecord> for FmpHttpInstitutionalOwnershipFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "institutional_ownership";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/institutional-holder/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        fmp_get(&url, &[], "fmp institutional_ownership").await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<OwnershipRecord>> {
        let entries: Vec<FmpInstitutionalHolderRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp institutional_ownership parse_json: {e}")))?;
        let rows = entries
            .into_iter()
            .filter_map(|entry| {
                let holder = entry.holder.filter(|h| !h.trim().is_empty())?;
                Some(OwnershipRecord {
                    symbol: query.symbol.clone(),
                    kind: "institutional".to_string(),
                    holder: Some(holder),
                    relationship: None,
                    date: entry.date_reported,
                    transaction_type: None,
                    shares: entry.shares,
                    value: entry.change,
                    percentage: None,
                })
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// FmpHttpGovernmentTradesFetcher — /v4/senate-trading → OwnershipRecord
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Production FMP government (senate) trading fetcher, normalized to
    /// [`OwnershipRecord`] (`kind = "government_trade"`).
    ///
    /// Calls `/v4/senate-trading?symbol=` and maps each disclosed congressional
    /// trade to an [`OwnershipRecord`].
    pub FmpHttpGovernmentTradesFetcher,
    BASE_URL
);

/// Wire shape for a `/v4/senate-trading` entry.
#[derive(Deserialize)]
struct FmpSenateTradeRaw {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    representative: Option<String>,
    #[serde(default)]
    office: Option<String>,
    #[serde(rename = "transactionDate", default)]
    transaction_date: Option<String>,
    #[serde(rename = "type", default)]
    transaction_type: Option<String>,
    #[serde(default)]
    amount: Option<String>,
}

#[async_trait]
impl Fetcher<FmpSymbolQuery, OwnershipRecord> for FmpHttpGovernmentTradesFetcher {
    const PROVIDER: &'static str = "fmp";
    const ENDPOINT: &'static str = "government_trades";

    fn transform_query(params: Value) -> Result<FmpSymbolQuery> {
        symbol_query(&params)
    }

    async fn extract_data(&self, query: &FmpSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        // The senate-trading endpoint lives at the v4 root, not v3.
        let url = format!("{}/senate-trading", peers_base(self.base_url()));
        fmp_get(
            &url,
            &[("symbol", query.symbol.clone())],
            "fmp government_trades",
        )
        .await
    }

    fn transform_data(&self, query: &FmpSymbolQuery, raw: Bytes) -> Result<Vec<OwnershipRecord>> {
        let entries: Vec<FmpSenateTradeRaw> = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("fmp government_trades parse_json: {e}")))?;
        let rows = entries
            .into_iter()
            .map(|entry| {
                // FMP reports the traded amount as a bucket string (e.g.
                // "$1,001 - $15,000"); surface the lower bound as a value hint.
                let value = entry.amount.as_deref().and_then(parse_amount_lower_bound);
                OwnershipRecord {
                    symbol: entry.symbol.unwrap_or_else(|| query.symbol.clone()),
                    kind: "government_trade".to_string(),
                    holder: entry.representative,
                    relationship: entry.office,
                    date: entry.transaction_date,
                    transaction_type: entry.transaction_type,
                    shares: None,
                    value,
                    percentage: None,
                }
            })
            .collect();
        Ok(rows)
    }
}

/// Parse the lower bound of an FMP amount bucket like `"$1,001 - $15,000"` into a
/// number, returning `None` when no leading numeric portion is present.
fn parse_amount_lower_bound(amount: &str) -> Option<f64> {
    let first = amount.split('-').next().unwrap_or(amount);
    let cleaned: String = first
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        cleaned.parse::<f64>().ok()
    }
}

// ---------------------------------------------------------------------------
// Shared query-parameter parsing helpers
// ---------------------------------------------------------------------------

/// Extract and require a `symbol` (or `ticker`) string from a query params blob.
fn symbol_param(params: &Value) -> Result<&str> {
    params
        .get("symbol")
        .or_else(|| params.get("ticker"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidQuery("fmp symbol must be a string".to_string()))
}

/// Parse the optional `statement` discriminator, defaulting to income.
fn statement_param(params: &Value) -> Result<FmpStatement> {
    match params.get("statement").and_then(Value::as_str) {
        None | Some("income") => Ok(FmpStatement::Income),
        Some("balance") => Ok(FmpStatement::Balance),
        Some("cashflow") => Ok(FmpStatement::Cashflow),
        Some(other) => Err(Error::InvalidQuery(format!(
            "fmp unknown statement type: {other}"
        ))),
    }
}

/// Parse the optional `limit`, defaulting to `default`.
fn limit_param(params: &Value, default: u32) -> Result<u32> {
    params
        .get("limit")
        .and_then(Value::as_u64)
        .map(u32::try_from)
        .transpose()
        .map_err(|e| Error::InvalidQuery(format!("fmp limit too large: {e}")))
        .map(|limit| limit.unwrap_or(default))
}

/// Build a validated [`FmpFundamentalQuery`] from a params blob.
fn fundamental_query(params: &Value) -> Result<FmpFundamentalQuery> {
    let symbol = symbol_param(params)?;
    let period = crate::FmpPeriod::from_param(params.get("period").and_then(Value::as_str));
    let limit = limit_param(params, 5)?;
    FmpFundamentalQuery::new(symbol, period, limit).map_err(|e| Error::InvalidQuery(e.to_string()))
}

/// Build a validated [`FmpSymbolQuery`] from a params blob.
fn symbol_query(params: &Value) -> Result<FmpSymbolQuery> {
    let symbol = symbol_param(params)?;
    FmpSymbolQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
}
