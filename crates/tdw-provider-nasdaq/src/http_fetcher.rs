#![cfg(feature = "http")]
//! Real NASDAQ Data Link backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to the NASDAQ Data Link
//! `datasets/{database}/{dataset}/data` endpoint directly via
//! `reqwest`. Live calls require `TDW_NASDAQ_API_KEY`; the live
//! integration test is additionally gated by `TDW_NASDAQ_LIVE=1` so
//! unattended CI stays offline.

use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{BASE_URL, NasdaqDataRow, NasdaqDatasetQuery, dataset_request};

const API_KEY_ENV: &str = "TDW_NASDAQ_API_KEY";
const USER_AGENT: &str = "tdw-provider-nasdaq/0.1";

/// Top-level API envelope for the dataset data endpoint.
#[derive(Deserialize)]
struct NasdaqEnvelope {
    #[serde(default)]
    dataset_data: Option<NasdaqDatasetData>,
    /// Present when the API returns an error.
    #[serde(default)]
    quandl_error: Option<NasdaqError>,
}

#[derive(Deserialize)]
struct NasdaqDatasetData {
    #[serde(default)]
    column_names: Vec<String>,
    #[serde(default)]
    data: Vec<Vec<Value>>,
}

#[derive(Deserialize)]
struct NasdaqError {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

/// Provider specification for the NASDAQ Data Link dataset fetcher.
pub struct NasdaqDatasetSpec;

impl ProviderSpec for NasdaqDatasetSpec {
    const PROVIDER: &'static str = "nasdaq";
    const ENDPOINT: &'static str = "datasets";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "nasdaq client";
    const SEND_ERR: &'static str = "nasdaq extract_data";
    const RETURNED_ERR: &'static str = "nasdaq extract_data returned";
    const READ_BODY_ERR: &'static str = "nasdaq read body";

    type Query = NasdaqDatasetQuery;
    type Data = NasdaqDataRow;

    fn transform_query(params: Value) -> Result<NasdaqDatasetQuery> {
        let database = params
            .get("database")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("nasdaq database must be a string".to_string()))?;
        let dataset = params
            .get("dataset")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("nasdaq dataset must be a string".to_string()))?;

        let mut query = NasdaqDatasetQuery::new(database, dataset)
            .map_err(|error| Error::InvalidQuery(error.to_string()))?;

        if let Some(start) = params.get("start_date").and_then(Value::as_str) {
            query = query
                .with_start_date(start)
                .map_err(|error| Error::InvalidQuery(error.to_string()))?;
        }
        if let Some(end) = params.get("end_date").and_then(Value::as_str) {
            query = query
                .with_end_date(end)
                .map_err(|error| Error::InvalidQuery(error.to_string()))?;
        }

        Ok(query)
    }

    fn build_request(
        base_url: &str,
        query: &NasdaqDatasetQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        dataset_request(&query.database, &query.dataset, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "nasdaq")?;

        let endpoint = format!(
            "{}/datasets/{}/{}/data",
            base_url.trim_end_matches('/'),
            query.database,
            query.dataset,
        );

        let mut query_params: Vec<(&str, String)> = vec![("api_key", api_key)];
        if let Some(start) = &query.start_date {
            query_params.push(("start_date", start.clone()));
        }
        if let Some(end) = &query.end_date {
            query_params.push(("end_date", end.clone()));
        }

        Ok(client.get(&endpoint).query(&query_params))
    }

    fn transform_data(query: &NasdaqDatasetQuery, raw: Bytes) -> Result<Vec<NasdaqDataRow>> {
        let envelope: NasdaqEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("nasdaq parse_json: {error}")))?;

        if let Some(api_error) = envelope.quandl_error {
            let code = api_error.code.unwrap_or_default();
            let message = api_error.message.unwrap_or_default();
            return Err(Error::Provider(format!(
                "nasdaq api error {code}: {message}"
            )));
        }

        let dataset_data = envelope.dataset_data.ok_or_else(|| {
            Error::Provider("nasdaq response missing dataset_data field".to_string())
        })?;

        let mut rows = Vec::with_capacity(dataset_data.data.len());
        for data_row in dataset_data.data {
            rows.push(NasdaqDataRow {
                database: query.database.clone(),
                dataset: query.dataset.clone(),
                column_names: dataset_data.column_names.clone(),
                values: data_row,
            });
        }
        Ok(rows)
    }
}

/// Production NASDAQ Data Link dataset fetcher.
pub type NasdaqHttpDatasetFetcher = HttpFetcher<NasdaqDatasetSpec>;

// ---------------------------------------------------------------------------
// Catalog-facing calendar fetcher -> tdw_domain::CalendarEvent
//
// The dataset fetcher above targets the keyed NASDAQ Data Link (Quandl)
// `datasets` API and has no calendar plumbing. The namespaced endpoint catalog
// routes `equity/calendar/{dividends,earnings,ipo}` are served by the public,
// keyless NASDAQ market-calendar API instead. This fetcher reuses the crate's
// `normalize_date` path-safety guard for the `date` query parameter and emits
// the standardized `tdw_domain::CalendarEvent`. One fetcher serves all three
// calendars; the calendar discriminator is injected per dispatch binding,
// mirroring the FRED command-injection pattern.
//
// Public NASDAQ calendar API (no key):
//   GET https://api.nasdaq.com/api/calendar/dividends?date=YYYY-MM-DD
//   GET https://api.nasdaq.com/api/calendar/earnings?date=YYYY-MM-DD
//   GET https://api.nasdaq.com/api/ipo/calendar?date=YYYY-MM
// ---------------------------------------------------------------------------

use async_trait::async_trait;
use tdw_core::{Credentials, Fetcher};
use tdw_domain::CalendarEvent;

use crate::{CALENDAR_BASE_URL, normalize_date};

/// Which NASDAQ market calendar a [`NasdaqCalendarQuery`] targets.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NasdaqCalendarKind {
    /// Dividend calendar (`equity/calendar/dividends`).
    Dividends,
    /// Earnings calendar (`equity/calendar/earnings`).
    Earnings,
    /// IPO calendar (`equity/calendar/ipo`).
    Ipo,
    /// Corporate-events calendar (`equity/calendar/events`).
    Events,
}

impl NasdaqCalendarKind {
    /// The `CalendarEvent::kind` discriminator emitted for this calendar.
    const fn event_kind(self) -> &'static str {
        match self {
            Self::Dividends => "dividend",
            Self::Earnings => "earnings",
            Self::Ipo => "ipo",
            Self::Events => "event",
        }
    }

    /// The `snake_case` serde value for the `calendar` query field, so the
    /// dispatcher can inject it for the per-calendar dispatch binding.
    #[must_use]
    pub const fn as_query_str(self) -> &'static str {
        match self {
            Self::Dividends => "dividends",
            Self::Earnings => "earnings",
            Self::Ipo => "ipo",
            Self::Events => "events",
        }
    }
}

/// Validated query for the standardized NASDAQ calendar routes.
///
/// Carries the calendar discriminator (injected per dispatch binding) plus an
/// optional `date` filter. The `date` is validated through the crate's
/// `normalize_date` guard so it cannot inject extra query parameters.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct NasdaqCalendarQuery {
    /// Which calendar to fetch.
    pub calendar: NasdaqCalendarKind,
    /// Optional `YYYY-MM-DD` date filter (the calendar day to list).
    #[serde(default)]
    pub date: Option<String>,
}

impl NasdaqCalendarQuery {
    /// Build a calendar query from a raw caller payload.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] when `calendar` is missing/invalid or the
    /// optional `date` is not `YYYY-MM-DD`.
    pub fn from_value(params: &Value) -> Result<Self> {
        let calendar = params
            .get("calendar")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("nasdaq calendar must be a string".to_string()))?;
        let calendar: NasdaqCalendarKind =
            serde_json::from_value(Value::String(calendar.to_string()))
                .map_err(|e| Error::InvalidQuery(format!("nasdaq calendar: {e}")))?;
        let date = match params.get("date").and_then(Value::as_str) {
            Some(date) => {
                Some(normalize_date(date).map_err(|e| Error::InvalidQuery(e.to_string()))?)
            }
            None => None,
        };
        Ok(Self { calendar, date })
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production NASDAQ market-calendar fetcher (catalog-facing).
    ///
    /// Standardizes `equity/calendar/{dividends,earnings,ipo}`, normalized to
    /// [`tdw_domain::CalendarEvent`]. Talks to the public, keyless NASDAQ
    /// calendar API; reuses the crate's `normalize_date` path guard.
    pub NasdaqHttpCalendarFetcher,
    CALENDAR_BASE_URL
);

#[async_trait]
impl Fetcher<NasdaqCalendarQuery, CalendarEvent> for NasdaqHttpCalendarFetcher {
    const PROVIDER: &'static str = "nasdaq";
    const ENDPOINT: &'static str = "calendar";

    fn transform_query(params: Value) -> Result<NasdaqCalendarQuery> {
        NasdaqCalendarQuery::from_value(&params)
    }

    async fn extract_data(
        &self,
        query: &NasdaqCalendarQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let path = match query.calendar {
            NasdaqCalendarKind::Dividends => "calendar/dividends",
            NasdaqCalendarKind::Earnings => "calendar/earnings",
            NasdaqCalendarKind::Ipo => "ipo/calendar",
            NasdaqCalendarKind::Events => "calendar/events",
        };
        let endpoint = format!("{}/{path}", self.base_url().trim_end_matches('/'));
        let mut query_params: Vec<(&str, String)> = Vec::new();
        if let Some(date) = &query.date {
            query_params.push(("date", date.clone()));
        }
        // The public NASDAQ API rejects requests without a browser-like
        // User-Agent; the shared client sets ours.
        let client = tdw_core::http_support::build_client(USER_AGENT, "nasdaq client")?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("nasdaq calendar request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "nasdaq calendar returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("nasdaq calendar read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &NasdaqCalendarQuery,
        raw: Bytes,
    ) -> Result<Vec<CalendarEvent>> {
        let value: Value = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("nasdaq calendar parse_json: {e}")))?;
        let rows = calendar_rows(&value);
        Ok(rows
            .iter()
            .filter_map(|row| decode_calendar_row(query.calendar, row))
            .collect())
    }
}

/// Locate the array of calendar rows in a NASDAQ calendar response.
///
/// The feeds nest rows differently per calendar (`data.rows`,
/// `data.calendar.rows`, or `data.upcoming.upcomingTable.rows`); this probes the
/// known shapes in order and returns the first array found.
fn calendar_rows(value: &Value) -> Vec<Value> {
    const POINTERS: &[&str] = &[
        "/data/rows",
        "/data/calendar/rows",
        "/data/upcoming/upcomingTable/rows",
        "/data/priced/rows",
    ];
    for pointer in POINTERS {
        if let Some(rows) = value.pointer(pointer).and_then(Value::as_array) {
            return rows.clone();
        }
    }
    Vec::new()
}

/// Decode one calendar row into a [`CalendarEvent`], or `None` when it carries
/// no usable symbol. Field names follow the public NASDAQ calendar payloads.
fn decode_calendar_row(kind: NasdaqCalendarKind, row: &Value) -> Option<CalendarEvent> {
    let symbol = str_field(row, &["symbol", "proposedTickerSymbol", "ticker"])?;
    let mut event = CalendarEvent {
        kind: kind.event_kind().to_string(),
        symbol,
        name: str_field(row, &["companyName", "name", "company"]),
        date: str_field(row, &["date", "expectedPriceDate", "dividend_Ex_Date"]),
        dividend: None,
        payment_date: None,
        record_date: None,
        eps_estimate: None,
        fiscal_period: None,
        price: None,
        shares: None,
        exchange: None,
    };
    match kind {
        NasdaqCalendarKind::Dividends => {
            event.dividend = num_field(row, &["dividend_Rate", "amount"]);
            event.payment_date = str_field(row, &["payment_Date", "paymentDate"]);
            event.record_date = str_field(row, &["record_Date", "recordDate"]);
        }
        NasdaqCalendarKind::Earnings => {
            event.eps_estimate = num_field(row, &["epsForecast", "eps", "consensusForecast"]);
            event.fiscal_period = str_field(row, &["fiscalQuarterEnding", "fiscalPeriod"]);
        }
        NasdaqCalendarKind::Ipo => {
            event.price = num_field(row, &["proposedSharePrice", "price"]);
            event.shares = num_field(row, &["sharesOffered", "shares"]);
            event.exchange = str_field(row, &["proposedExchange", "exchange", "market"]);
        }
        NasdaqCalendarKind::Events => {
            // Corporate-events rows carry a free-text event title; surface it via
            // `name` when the row has no separate company name.
            if event.name.is_none() {
                event.name = str_field(row, &["eventName", "event", "title"]);
            }
        }
    }
    Some(event)
}

/// First non-empty string among `keys` in `row`.
fn str_field(row: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = row.get(*key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// First parseable number among `keys` in `row`, accepting both JSON numbers and
/// money-formatted strings (`"$1.25"`, `"1,250,000"`, `"N/A"` → `None`).
fn num_field(row: &Value, keys: &[&str]) -> Option<f64> {
    for key in keys {
        match row.get(*key) {
            Some(Value::Number(number)) => return number.as_f64(),
            Some(Value::String(text)) => {
                let cleaned: String = text
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
                    .collect();
                if let Ok(parsed) = cleaned.parse::<f64>() {
                    return Some(parsed);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Catalog-facing S&P 500 / Shiller multiples fetcher -> tdw_domain::Sp500Multiple
//
// The keyed NASDAQ Data Link (Quandl) `MULTPL` database publishes the classic
// long-run S&P 500 valuation multiples (Robert Shiller's CAPE, the trailing PE,
// dividend & earnings yields, etc.) as time-series datasets. This fetcher reuses
// the crate's `NasdaqDatasetSpec` request builder and `[Date, Value]` envelope
// parser verbatim, then maps each observation onto the standardized
// `tdw_domain::Sp500Multiple`. The `metric` param selects the dataset within the
// `MULTPL` database (default: the monthly Shiller CAPE).
//
// MULTPL database: <https://data.nasdaq.com/data/MULTPL>.
// ---------------------------------------------------------------------------

use tdw_domain::Sp500Multiple;

/// `MULTPL`-database dataset code for a requested S&P 500 multiple metric.
///
/// Maps the public `metric` param to the Data Link dataset within the keyless-to-
/// name but keyed-to-fetch `MULTPL` database. Unknown / missing metrics default
/// to the monthly Shiller CAPE PE ratio.
fn sp500_multiple_dataset(metric: &str) -> &'static str {
    match metric.trim().to_ascii_lowercase().as_str() {
        "sp500_pe_ratio_month" | "pe_ratio" | "pe" => "SP500_PE_RATIO_MONTH",
        "sp500_div_yield_month" | "dividend_yield" | "div_yield" => "SP500_DIV_YIELD_MONTH",
        "sp500_earnings_yield_month" | "earnings_yield" => "SP500_EARNINGS_YIELD_MONTH",
        "sp500_real_price_month" | "real_price" => "SP500_REAL_PRICE_MONTH",
        // Default: Robert Shiller's cyclically-adjusted PE (CAPE).
        _ => "SHILLER_PE_RATIO_MONTH",
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production NASDAQ Data Link S&P 500 / Shiller-multiples fetcher
    /// (catalog-facing).
    ///
    /// Standardizes `index/sp500_multiples`, normalized to
    /// [`tdw_domain::Sp500Multiple`]. Targets the keyed Data Link `MULTPL`
    /// database (requires `TDW_NASDAQ_API_KEY`); the `metric` param selects the
    /// dataset (default: the monthly Shiller CAPE PE ratio). Reuses
    /// [`NasdaqDatasetSpec`]'s request builder and `[Date, Value]` parser.
    pub NasdaqHttpSp500MultiplesFetcher,
    BASE_URL
);

/// Validated query for the standardized S&P 500 multiples route.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct NasdaqSp500MultiplesQuery {
    /// Lower-cased metric label (also the emitted `Sp500Multiple::metric`).
    pub metric: String,
    /// Underlying `MULTPL` dataset query (carries the optional date window).
    pub dataset: NasdaqDatasetQuery,
}

#[async_trait]
impl Fetcher<NasdaqSp500MultiplesQuery, Sp500Multiple> for NasdaqHttpSp500MultiplesFetcher {
    const PROVIDER: &'static str = "nasdaq";
    const ENDPOINT: &'static str = "sp500_multiples";

    fn transform_query(params: Value) -> Result<NasdaqSp500MultiplesQuery> {
        let metric = params
            .get("metric")
            .and_then(Value::as_str)
            .unwrap_or("shiller_pe_ratio_month")
            .trim()
            .to_ascii_lowercase();
        let dataset_code = sp500_multiple_dataset(&metric);
        let mut dataset = NasdaqDatasetQuery::new("MULTPL", dataset_code)
            .map_err(|e| Error::InvalidQuery(e.to_string()))?;
        if let Some(start) = params.get("start_date").and_then(Value::as_str) {
            dataset = dataset
                .with_start_date(start)
                .map_err(|e| Error::InvalidQuery(e.to_string()))?;
        }
        if let Some(end) = params.get("end_date").and_then(Value::as_str) {
            dataset = dataset
                .with_end_date(end)
                .map_err(|e| Error::InvalidQuery(e.to_string()))?;
        }
        Ok(NasdaqSp500MultiplesQuery { metric, dataset })
    }

    async fn extract_data(
        &self,
        query: &NasdaqSp500MultiplesQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let client = tdw_core::http_support::build_client(USER_AGENT, "nasdaq client")?;
        let request = <NasdaqDatasetSpec as ProviderSpec>::build_request(
            self.base_url(),
            &query.dataset,
            &client,
        )?;
        let response = request
            .send()
            .await
            .map_err(|e| Error::Provider(format!("nasdaq sp500_multiples request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "nasdaq sp500_multiples returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("nasdaq sp500_multiples read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &NasdaqSp500MultiplesQuery,
        raw: Bytes,
    ) -> Result<Vec<Sp500Multiple>> {
        let rows = <NasdaqDatasetSpec as ProviderSpec>::transform_data(&query.dataset, raw)?;
        // Each MULTPL dataset row is a `[Date, Value]` pair (the column order the
        // Data Link `dataset_data.data` matrix reports). Map the leading date cell
        // and the trailing numeric cell onto the standardized model.
        let multiples = rows
            .into_iter()
            .filter_map(|row| {
                let date = row.values.first().and_then(Value::as_str)?.to_string();
                let value = row.values.get(1).and_then(value_as_f64);
                Some(Sp500Multiple {
                    metric: query.metric.clone(),
                    date,
                    value,
                })
            })
            .collect();
        Ok(multiples)
    }
}

/// Parse a Data Link value cell as `f64`, accepting both JSON numbers and
/// numeric strings; returns `None` for nulls / non-numeric cells.
fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Catalog-facing top-retail discovery fetcher -> tdw_domain::ScreenerRow
//
// The public, keyless NASDAQ market-activity API publishes a top-retail-traded
// list (`/api/market-activity/quotes/...`). This fetcher normalizes each entry
// onto the standardized `tdw_domain::ScreenerRow` (the same model the discovery
// screeners emit). It targets the public NASDAQ host (no key) and reuses the
// crate's browser-like `User-Agent`. Standardizes `equity/discovery/top_retail`.
// ---------------------------------------------------------------------------

use tdw_domain::ScreenerRow;

/// Parameter-free query for the keyless NASDAQ top-retail discovery route.
///
/// The feed lists the current top retail-traded symbols and takes no filter, so
/// the query carries no fields; it exists so the fetcher satisfies the
/// `Fetcher<Q, D>` contract with a `JsonSchema`-bearing query.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
pub struct NasdaqTopRetailQuery {}

tdw_core::provider_fetcher_struct!(
    /// Production NASDAQ top-retail discovery fetcher (catalog-facing).
    ///
    /// Standardizes `equity/discovery/top_retail`, normalized to
    /// [`tdw_domain::ScreenerRow`]. Talks to the public, keyless NASDAQ
    /// market-activity API.
    pub NasdaqHttpTopRetailFetcher,
    CALENDAR_BASE_URL
);

#[async_trait]
impl Fetcher<NasdaqTopRetailQuery, ScreenerRow> for NasdaqHttpTopRetailFetcher {
    const PROVIDER: &'static str = "nasdaq";
    const ENDPOINT: &'static str = "top_retail";

    fn transform_query(_params: Value) -> Result<NasdaqTopRetailQuery> {
        Ok(NasdaqTopRetailQuery {})
    }

    async fn extract_data(
        &self,
        _query: &NasdaqTopRetailQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let endpoint = format!(
            "{}/market-activity/quotes/top-retail",
            self.base_url().trim_end_matches('/'),
        );
        let client = tdw_core::http_support::build_client(USER_AGENT, "nasdaq client")?;
        let response = client
            .get(&endpoint)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("nasdaq top_retail request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "nasdaq top_retail returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("nasdaq top_retail read body: {e}")))
    }

    fn transform_data(
        &self,
        _query: &NasdaqTopRetailQuery,
        raw: Bytes,
    ) -> Result<Vec<ScreenerRow>> {
        let value: Value = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("nasdaq top_retail parse_json: {e}")))?;
        let rows = calendar_rows(&value);
        Ok(rows.iter().filter_map(decode_top_retail_row).collect())
    }
}

/// Decode one top-retail row into a [`ScreenerRow`], or `None` when it carries
/// no usable symbol. Field names follow the public NASDAQ market-activity
/// payloads (which reuse the `symbol`/`companyName`/`lastSalePrice` shape).
fn decode_top_retail_row(row: &Value) -> Option<ScreenerRow> {
    let symbol = str_field(row, &["symbol", "ticker"])?;
    Some(ScreenerRow {
        symbol,
        company_name: str_field(row, &["companyName", "name", "company"]),
        market_cap: num_field(row, &["marketCap", "marketcap"]),
        sector: str_field(row, &["sector"]),
        industry: str_field(row, &["industry"]),
        beta: None,
        price: num_field(row, &["lastSalePrice", "lastsale", "price"]),
        last_annual_dividend: None,
        volume: num_field(row, &["volume"]),
        exchange: str_field(row, &["exchange", "market"]),
        exchange_short_name: None,
        country: None,
        is_etf: None,
        is_actively_trading: None,
    })
}
