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
//!
//! Live calls require `TDW_FMP_API_KEY`. The live integration test is
//! additionally gated by `TDW_FMP_LIVE=1` so unattended CI stays offline.

#![cfg(feature = "http")]

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Map;
use tdw_core::http_support::prelude::*;
use tdw_domain::{
    CompanyProfile, CorporateAction, Estimate, FinancialStatement, Instrument, KeyMetrics,
    MarketDataBar, Ohlcv, QuoteSnapshot, Ratios, StatementKind, TimeGranularity,
};

use crate::{
    API_KEY_ENV, BASE_URL, FmpError, FmpFundamentalQuery, FmpFundamentalsQuery, FmpHistoricalQuery,
    FmpIncomeRow, FmpQuoteQuery, FmpStatement, FmpStatementQuery, FmpSymbolQuery,
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
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| FmpError::Provider(format!("fmp client build: {e}")))
}

fn api_key() -> std::result::Result<String, FmpError> {
    let key = std::env::var(API_KEY_ENV)
        .map_err(|_| FmpError::Provider(format!("{API_KEY_ENV} not set")))?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(FmpError::Provider(format!(
            "{API_KEY_ENV} must not be empty"
        )));
    }
    Ok(key)
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
        let api_key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
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
        let api_key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
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
        let api_key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
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
    let api_key = api_key().map_err(|e| Error::Provider(e.to_string()))?;
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
