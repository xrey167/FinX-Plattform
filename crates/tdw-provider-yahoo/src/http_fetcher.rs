//! Real Yahoo Finance backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to Yahoo Finance's v8 chart
//! endpoint directly via `reqwest` — no SDK needed. The endpoint is
//! unauthenticated for delayed equity historical data; rate limits
//! apply but are generous enough for typical batch use. The live
//! integration test is additionally gated by the env var
//! `TDW_YAHOO_LIVE=1` so unattended CI runs do not hit Yahoo by
//! accident.
//!
//! Returned bars are dated to the trading day implied by the
//! Unix timestamp Yahoo emits; the in-crate `unix_to_iso_date`
//! helper handles the conversion without pulling chrono / time / jiff
//! as workspace dependencies just for this one call site.

use serde::Deserialize;
use tdw_core::http_support::prelude::*;
use tdw_domain::{
    CompanyProfile, CorporateAction, EquityHistoricalData, Estimate, FuturesCurvePoint,
    OptionContract, OwnershipRecord, PricePerformance, QuoteSnapshot,
};
use tdw_provider_fileset::EquityHistoricalQuery;

use crate::{BASE_URL, YahooSymbolQuery};

const DEFAULT_INTERVAL: &str = "1d";
const DEFAULT_RANGE: &str = "5d";
const USER_AGENT: &str = "tdw-provider-yahoo/0.1";

/// Build a `reqwest` client with the Yahoo user-agent, mapping errors to
/// [`Error::Provider`] with a per-call-site `ctx` prefix.
fn yahoo_client(ctx: &str) -> Result<Client> {
    tdw_core::http_support::build_client(USER_AGENT, ctx)
}

/// Cookie-issuing endpoint for the crumb handshake. Returns 404 but sets the
/// session cookie the crumb endpoint requires.
const CRUMB_COOKIE_URL: &str = "https://fc.yahoo.com/";
/// Crumb endpoint; needs the session cookie from [`CRUMB_COOKIE_URL`].
const CRUMB_URL: &str = "https://query1.finance.yahoo.com/v1/test/getcrumb";
/// Browser-like user agent for the crumb handshake and crumb-authenticated
/// retries: Yahoo answers 429 to crumb requests from non-browser UAs, so the
/// crate's own UA cannot be used on this path.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Acquire a Yahoo session cookie + crumb pair. The v10 `quoteSummary` and
/// v7 quote/options endpoints reject anonymous requests with
/// 401 "Invalid Crumb"; this two-step handshake (cookie, then crumb) is what
/// browsers do implicitly.
async fn yahoo_handshake(client: &Client, ctx: &str) -> Result<(String, String)> {
    let response = client
        .get(CRUMB_COOKIE_URL)
        .header(reqwest::header::USER_AGENT, BROWSER_USER_AGENT)
        .send()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} crumb cookie: {error}")))?;
    // fc.yahoo.com answers 404 — only the Set-Cookie headers matter.
    let cookie = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    if cookie.is_empty() {
        return Err(Error::Provider(format!(
            "{ctx} crumb cookie: no session cookie issued"
        )));
    }
    let response = client
        .get(CRUMB_URL)
        .header(reqwest::header::USER_AGENT, BROWSER_USER_AGENT)
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} crumb fetch: {error}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "{ctx} crumb fetch returned {status}: {body}"
        )));
    }
    let crumb = response
        .text()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} crumb read: {error}")))?;
    if crumb.is_empty() || crumb.contains('{') {
        return Err(Error::Provider(format!(
            "{ctx} crumb fetch returned an error body: {crumb}"
        )));
    }
    Ok((cookie, crumb))
}

/// Issue a GET to `url`, returning the raw body bytes or an [`Error::Provider`]
/// carrying the failing status + body. `ctx` prefixes every error message so
/// the failing endpoint is identifiable.
///
/// On 401/403 the request is retried once with a freshly acquired
/// cookie + crumb (see [`yahoo_handshake`]). The handshake is strictly lazy so
/// offline cassette/mock tests never touch the network.
async fn yahoo_get(client: &Client, url: &str, ctx: &str) -> Result<Bytes> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} extract_data: {error}")))?;
    if !response.status().is_success() {
        let status = response.status();
        if matches!(status.as_u16(), 401 | 403) {
            let (cookie, crumb) = yahoo_handshake(client, ctx).await?;
            let retry = client
                .get(url)
                .query(&[("crumb", crumb.as_str())])
                .header(reqwest::header::USER_AGENT, BROWSER_USER_AGENT)
                .header(reqwest::header::COOKIE, &cookie)
                .send()
                .await
                .map_err(|error| Error::Provider(format!("{ctx} extract_data: {error}")))?;
            if !retry.status().is_success() {
                let status = retry.status();
                let body = retry.text().await.unwrap_or_default();
                return Err(Error::Provider(format!("{ctx} returned {status}: {body}")));
            }
            return retry
                .bytes()
                .await
                .map_err(|error| Error::Provider(format!("{ctx} read body: {error}")));
        }
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!("{ctx} returned {status}: {body}")));
    }
    response
        .bytes()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} read body: {error}")))
}

/// Production Yahoo Finance historical fetcher.
#[derive(Clone, Debug)]
pub struct YahooHttpEquityHistoricalFetcher {
    base_url: String,
    interval: String,
    range: String,
}

impl Default for YahooHttpEquityHistoricalFetcher {
    fn default() -> Self {
        Self {
            base_url: "https://query1.finance.yahoo.com".to_string(),
            interval: DEFAULT_INTERVAL.to_string(),
            range: DEFAULT_RANGE.to_string(),
        }
    }
}

impl YahooHttpEquityHistoricalFetcher {
    /// Override the Yahoo base URL — useful for pointing the fetcher
    /// at a recorded-cassette HTTP server during integration tests.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Override the chart interval (e.g. `"1d"`, `"1h"`, `"5m"`).
    pub fn with_interval(mut self, interval: impl Into<String>) -> Self {
        self.interval = interval.into();
        self
    }

    /// Override the chart range (e.g. `"5d"`, `"1mo"`, `"1y"`).
    pub fn with_range(mut self, range: impl Into<String>) -> Self {
        self.range = range.into();
        self
    }

    /// Registry entry advertised under the canonical `yahoo` provider
    /// name.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[derive(Deserialize)]
struct ChartEnvelope {
    chart: ChartResult,
}

#[derive(Deserialize)]
struct ChartResult {
    #[serde(default)]
    result: Vec<ChartSeries>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct ChartSeries {
    #[serde(default)]
    timestamp: Vec<i64>,
    indicators: ChartIndicators,
}

#[derive(Deserialize)]
struct ChartIndicators {
    quote: Vec<ChartQuote>,
}

#[derive(Deserialize)]
struct ChartQuote {
    #[serde(default)]
    open: Vec<Option<f64>>,
    #[serde(default)]
    high: Vec<Option<f64>>,
    #[serde(default)]
    low: Vec<Option<f64>>,
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<i64>>,
}

/// Build the Yahoo v8 chart URL, honoring an absolute `start`/`end` date
/// window when present and otherwise falling back to the relative `range`.
///
/// Yahoo accepts either `range=` (relative) or `period1=`/`period2=` (absolute
/// epoch seconds), not a meaningful mix. `period2` is exclusive on the API, so
/// a present `end` date is pushed forward one day to include that day's bar; an
/// absent `end` runs to `now_unix`, and an absent `start` runs from the epoch
/// (Yahoo clamps to the symbol's listing date).
fn build_chart_url(
    base_url: &str,
    symbol: &str,
    interval: &str,
    range: &str,
    start: Option<tdw_core::Date>,
    end: Option<tdw_core::Date>,
    now_unix: i64,
) -> String {
    if start.is_none() && end.is_none() {
        return format!("{base_url}/v8/finance/chart/{symbol}?interval={interval}&range={range}");
    }
    let to_epoch = |d: tdw_core::Date| {
        tdw_core::date::civil_to_unix_seconds(
            i64::from(d.year()),
            u32::from(d.month()),
            u32::from(d.day()),
        )
    };
    let period1 = start.map_or(0, to_epoch);
    let period2 = end.map_or(now_unix, |d| to_epoch(d) + 86_400);
    format!(
        "{base_url}/v8/finance/chart/{symbol}?interval={interval}&period1={period1}&period2={period2}"
    )
}

#[async_trait]
impl Fetcher<EquityHistoricalQuery, EquityHistoricalData> for YahooHttpEquityHistoricalFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> Result<EquityHistoricalQuery> {
        // Delegate to the fileset fetcher's symbol validation /
        // normalization so we stay consistent with the rest of the
        // provider surface.
        tdw_provider_fileset::FilesetEquityHistoricalFetcher::transform_query(params)
    }

    async fn extract_data(
        &self,
        query: &EquityHistoricalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        // Prefer the caller-supplied interval (parsed once by the shared
        // `StandardParams` normalization) over the fetcher's configured
        // default, so `interval=` in the request payload reaches Yahoo.
        let interval = query.params.interval.as_token();
        let interval = if interval == DEFAULT_INTERVAL {
            self.interval.as_str()
        } else {
            interval
        };
        // Honor a caller-supplied date window. Yahoo's v8 chart accepts either
        // a relative `range` or an absolute `period1`/`period2` epoch-second
        // pair; when the request carries start/end dates we must send the
        // latter, otherwise the window is silently dropped and only the
        // configured `self.range` is returned.
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            });
        let url = build_chart_url(
            &self.base_url,
            &query.symbol,
            interval,
            &self.range,
            query.params.start_date,
            query.params.end_date,
            now_unix,
        );
        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("yahoo client: {error}")))?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("yahoo extract_data: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "yahoo extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("yahoo read body: {error}")))
    }

    fn transform_data(
        &self,
        query: &EquityHistoricalQuery,
        raw: Bytes,
    ) -> Result<Vec<EquityHistoricalData>> {
        // Drop bars where Yahoo emitted nulls (Yahoo sometimes includes a
        // "current" bar with all-null fields when the requested range overlaps
        // an open session). Shared with the futures-historical fetcher.
        decode_chart_bars(&raw, &query.symbol, "yahoo")
    }
}

// ===========================================================================
// L2.4 expansion: profile / quote / performance / dividends / share_statistics
// / consensus / futures (historical + curve) / options chains.
//
// All endpoints below use Yahoo's documented public JSON APIs (v7 quote,
// v8 chart, v10 quoteSummary, v7 options) — no API key required. Each fetcher
// normalizes to a `tdw-domain` L1.4 model where one applies, or to a small
// crate-local row type for the two shapes (price performance, futures curve)
// that have no L1.4 equivalent yet.
// ===========================================================================

// ---------------------------------------------------------------------------
// v10 quoteSummary envelope (shared by profile / share_statistics / consensus
// / performance).
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct QuoteSummaryEnvelope {
    #[serde(rename = "quoteSummary")]
    quote_summary: QuoteSummaryBody,
}

#[derive(Deserialize)]
struct QuoteSummaryBody {
    #[serde(default)]
    result: Vec<QuoteSummaryResult>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize, Default)]
struct QuoteSummaryResult {
    #[serde(rename = "assetProfile", default)]
    asset_profile: Option<AssetProfile>,
    #[serde(default)]
    price: Option<PriceModule>,
    #[serde(rename = "defaultKeyStatistics", default)]
    key_statistics: Option<KeyStatistics>,
    #[serde(rename = "financialData", default)]
    financial_data: Option<FinancialData>,
}

/// Yahoo wraps most numbers as `{ "raw": <f64>, "fmt": "..." }`.
#[derive(Deserialize, Default, Clone, Copy)]
struct RawNum {
    #[serde(default)]
    raw: Option<f64>,
}

impl RawNum {
    const fn value(self) -> Option<f64> {
        self.raw
    }
}

#[derive(Deserialize, Default)]
struct AssetProfile {
    #[serde(default)]
    sector: Option<String>,
    #[serde(default)]
    website: Option<String>,
}

#[derive(Deserialize, Default)]
struct PriceModule {
    #[serde(rename = "shortName", default)]
    short_name: Option<String>,
    #[serde(rename = "longName", default)]
    long_name: Option<String>,
    #[serde(default)]
    currency: Option<String>,
    #[serde(rename = "exchangeName", default)]
    exchange_name: Option<String>,
    #[serde(rename = "marketCap", default)]
    market_cap: RawNum,
    #[serde(rename = "regularMarketPrice", default)]
    regular_market_price: RawNum,
    #[serde(rename = "regularMarketPreviousClose", default)]
    regular_market_previous_close: RawNum,
}

#[derive(Deserialize, Default)]
struct KeyStatistics {
    #[serde(rename = "sharesOutstanding", default)]
    shares_outstanding: RawNum,
    #[serde(rename = "floatShares", default)]
    float_shares: RawNum,
    #[serde(rename = "heldPercentInsiders", default)]
    held_percent_insiders: RawNum,
    #[serde(rename = "heldPercentInstitutions", default)]
    held_percent_institutions: RawNum,
    #[serde(rename = "52WeekChange", default)]
    fifty_two_week_change: RawNum,
}

#[derive(Deserialize, Default)]
struct FinancialData {
    #[serde(rename = "targetMeanPrice", default)]
    target_mean_price: RawNum,
    #[serde(rename = "targetLowPrice", default)]
    target_low_price: RawNum,
    #[serde(rename = "targetHighPrice", default)]
    target_high_price: RawNum,
    #[serde(rename = "numberOfAnalystOpinions", default)]
    number_of_analyst_opinions: RawNum,
    #[serde(rename = "recommendationKey", default)]
    recommendation_key: Option<String>,
    #[serde(rename = "financialCurrency", default)]
    financial_currency: Option<String>,
    #[serde(rename = "currentPrice", default)]
    current_price: RawNum,
}

/// Decode a v10 `quoteSummary` envelope, returning the first (and only) result
/// block. Shared by every quoteSummary-backed fetcher's `transform_data`.
fn parse_quote_summary(raw: &Bytes, ctx: &str) -> Result<QuoteSummaryResult> {
    let envelope: QuoteSummaryEnvelope = serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("{ctx} parse_json: {error}")))?;
    if let Some(error) = envelope.quote_summary.error {
        return Err(Error::Provider(format!("{ctx} error: {error}")));
    }
    envelope
        .quote_summary
        .result
        .into_iter()
        .next()
        .ok_or_else(|| Error::Provider(format!("{ctx} missing result[0]")))
}

// ---------------------------------------------------------------------------
// YahooHttpProfileFetcher — equity/profile → CompanyProfile
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo company-profile fetcher (`v10 quoteSummary` `assetProfile`+`price`).
    pub YahooHttpProfileFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<YahooSymbolQuery, CompanyProfile> for YahooHttpProfileFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "equity_profile";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v10/finance/quoteSummary/{}?modules=assetProfile,price",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo profile")?;
        yahoo_get(&client, &url, "yahoo profile").await
    }

    fn transform_data(&self, query: &YahooSymbolQuery, raw: Bytes) -> Result<Vec<CompanyProfile>> {
        let result = parse_quote_summary(&raw, "yahoo profile")?;
        let price = result.price.unwrap_or_default();
        let profile = result.asset_profile.unwrap_or_default();
        let name = price
            .long_name
            .or(price.short_name)
            .unwrap_or_else(|| query.symbol.clone());
        // Yahoo reports market cap in absolute currency units; the domain field
        // is millions, so scale down. `industry`/`sector` ride along in the
        // exchange/logo-url slots only when present, otherwise stay blank.
        let market_cap_millions = price.market_cap.value().unwrap_or(0.0) / 1_000_000.0;
        Ok(vec![CompanyProfile {
            ticker: query.symbol.clone(),
            name,
            currency: price.currency.unwrap_or_default(),
            exchange: price.exchange_name.or(profile.sector).unwrap_or_default(),
            logo_url: profile.website.unwrap_or_default(),
            market_cap_millions,
        }])
    }
}

// ---------------------------------------------------------------------------
// YahooHttpQuoteFetcher — equity/price/quote → QuoteSnapshot
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo current-quote fetcher (`v7 quote`).
    pub YahooHttpQuoteFetcher,
    BASE_URL
);

#[derive(Deserialize)]
struct QuoteV7Envelope {
    #[serde(rename = "quoteResponse")]
    quote_response: QuoteV7Body,
}

#[derive(Deserialize)]
struct QuoteV7Body {
    #[serde(default)]
    result: Vec<QuoteV7Row>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct QuoteV7Row {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "regularMarketPrice", default)]
    regular_market_price: f64,
    #[serde(rename = "regularMarketChange", default)]
    regular_market_change: f64,
    #[serde(rename = "regularMarketChangePercent", default)]
    regular_market_change_percent: f64,
    #[serde(rename = "regularMarketPreviousClose", default)]
    regular_market_previous_close: f64,
    #[serde(rename = "regularMarketTime", default)]
    regular_market_time: i64,
}

#[async_trait]
impl Fetcher<YahooSymbolQuery, QuoteSnapshot> for YahooHttpQuoteFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "equity_quote";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v7/finance/quote?symbols={}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo quote")?;
        yahoo_get(&client, &url, "yahoo quote").await
    }

    fn transform_data(&self, query: &YahooSymbolQuery, raw: Bytes) -> Result<Vec<QuoteSnapshot>> {
        let envelope: QuoteV7Envelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("yahoo quote parse_json: {error}")))?;
        if let Some(error) = envelope.quote_response.error {
            return Err(Error::Provider(format!("yahoo quote error: {error}")));
        }
        let rows = envelope
            .quote_response
            .result
            .into_iter()
            .map(|row| QuoteSnapshot {
                symbol: row.symbol.unwrap_or_else(|| query.symbol.clone()),
                current_price: row.regular_market_price,
                change: row.regular_market_change,
                change_percent: row.regular_market_change_percent,
                prev_close: row.regular_market_previous_close,
                // Yahoo returns seconds; the domain field is milliseconds.
                ts_ms: row.regular_market_time * 1_000,
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// YahooHttpPricePerformanceFetcher — equity/price/performance → PricePerformance
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo price-performance fetcher (`v10 quoteSummary` `price`+`summaryDetail`
    /// +`defaultKeyStatistics`).
    pub YahooHttpPricePerformanceFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<YahooSymbolQuery, PricePerformance> for YahooHttpPricePerformanceFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "price_performance";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v10/finance/quoteSummary/{}?modules=price,summaryDetail,defaultKeyStatistics",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo performance")?;
        yahoo_get(&client, &url, "yahoo performance").await
    }

    fn transform_data(
        &self,
        query: &YahooSymbolQuery,
        raw: Bytes,
    ) -> Result<Vec<PricePerformance>> {
        let result = parse_quote_summary(&raw, "yahoo performance")?;
        let price = result.price.unwrap_or_default();
        let stats = result.key_statistics.unwrap_or_default();
        let last = price.regular_market_price.value();
        // Only the returns this endpoint can produce honestly are reported:
        //   * `one_day` from the previous close (a true close-to-close return);
        //   * `one_year` from Yahoo's dedicated `52WeekChange` statistic.
        // The `summaryDetail` moving averages (50d/200d) and the 52-week range
        // are NOT prior prices, so deriving "1-month/3-month/YTD returns" from
        // them is wrong: a moving-average deviation is not a period return, and
        // `(last - 52wk_low) / 52wk_low` is structurally non-negative (so it has
        // the wrong sign for any name that is down on the year). Those periods
        // require historical bars (the `equity_historical` fetcher), so they are
        // reported as `None` here rather than a mislabeled deviation.
        let pct = |from: Option<f64>| match (last, from) {
            (Some(now), Some(base)) if base != 0.0 => Some((now - base) / base),
            _ => None,
        };
        Ok(vec![PricePerformance {
            symbol: query.symbol.clone(),
            price: last,
            one_day: pct(price.regular_market_previous_close.value()),
            one_week: None,
            one_month: None,
            three_month: None,
            ytd: None,
            one_year: stats.fifty_two_week_change.value(),
        }])
    }
}

// ---------------------------------------------------------------------------
// YahooHttpDividendsFetcher — equity/fundamental/dividends → CorporateAction
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo dividends fetcher (`v8 chart` with `events=div`).
    pub YahooHttpDividendsFetcher,
    BASE_URL
);

#[derive(Deserialize)]
struct DividendsEnvelope {
    chart: DividendsChart,
}

#[derive(Deserialize)]
struct DividendsChart {
    #[serde(default)]
    result: Vec<DividendsSeries>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct DividendsSeries {
    #[serde(default)]
    events: DividendsEvents,
    #[serde(default)]
    meta: DividendsMeta,
}

#[derive(Deserialize, Default)]
struct DividendsMeta {
    #[serde(default)]
    currency: Option<String>,
}

#[derive(Deserialize, Default)]
struct DividendsEvents {
    #[serde(default)]
    dividends: std::collections::BTreeMap<String, DividendEvent>,
}

#[derive(Deserialize)]
struct DividendEvent {
    #[serde(default)]
    amount: f64,
    #[serde(default)]
    date: i64,
}

#[async_trait]
impl Fetcher<YahooSymbolQuery, CorporateAction> for YahooHttpDividendsFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "dividends";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v8/finance/chart/{}?interval=1d&range=10y&events=div",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo dividends")?;
        yahoo_get(&client, &url, "yahoo dividends").await
    }

    fn transform_data(&self, query: &YahooSymbolQuery, raw: Bytes) -> Result<Vec<CorporateAction>> {
        let envelope: DividendsEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("yahoo dividends parse_json: {error}")))?;
        if let Some(error) = envelope.chart.error {
            return Err(Error::Provider(format!("yahoo dividends error: {error}")));
        }
        let Some(series) = envelope.chart.result.into_iter().next() else {
            return Ok(Vec::new());
        };
        let currency = series.meta.currency.unwrap_or_default();
        let mut rows: Vec<CorporateAction> = series
            .events
            .dividends
            .into_values()
            .map(|event| CorporateAction {
                symbol: query.symbol.clone(),
                ex_date: unix_to_iso_date(event.date),
                action_type: "dividend".to_string(),
                split_ratio: 0.0,
                cash_amount: event.amount,
                currency: currency.clone(),
            })
            .collect();
        rows.sort_by(|a, b| a.ex_date.cmp(&b.ex_date));
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// YahooHttpShareStatisticsFetcher — equity/ownership/share_statistics →
// OwnershipRecord
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo share-statistics fetcher (`v10 quoteSummary` `defaultKeyStatistics`).
    pub YahooHttpShareStatisticsFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<YahooSymbolQuery, OwnershipRecord> for YahooHttpShareStatisticsFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "share_statistics";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v10/finance/quoteSummary/{}?modules=defaultKeyStatistics",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo share_statistics")?;
        yahoo_get(&client, &url, "yahoo share_statistics").await
    }

    fn transform_data(&self, query: &YahooSymbolQuery, raw: Bytes) -> Result<Vec<OwnershipRecord>> {
        let result = parse_quote_summary(&raw, "yahoo share_statistics")?;
        let stats = result.key_statistics.unwrap_or_default();
        Ok(vec![OwnershipRecord {
            symbol: query.symbol.clone(),
            kind: "share_statistics".to_string(),
            holder: None,
            relationship: None,
            date: None,
            transaction_type: None,
            shares: stats
                .float_shares
                .value()
                .or_else(|| stats.shares_outstanding.value()),
            value: stats.shares_outstanding.value(),
            // Surface the larger of insider / institution ownership percent as
            // the headline percentage; both ride along scaled to a fraction.
            percentage: stats
                .held_percent_institutions
                .value()
                .or_else(|| stats.held_percent_insiders.value()),
        }])
    }
}

// ---------------------------------------------------------------------------
// YahooHttpConsensusFetcher — equity/estimates/consensus → Estimate
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo analyst-consensus / price-target fetcher (`v10 quoteSummary`
    /// `financialData`).
    pub YahooHttpConsensusFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<YahooSymbolQuery, Estimate> for YahooHttpConsensusFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "analyst_consensus";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v10/finance/quoteSummary/{}?modules=financialData",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo consensus")?;
        yahoo_get(&client, &url, "yahoo consensus").await
    }

    fn transform_data(&self, query: &YahooSymbolQuery, raw: Bytes) -> Result<Vec<Estimate>> {
        let result = parse_quote_summary(&raw, "yahoo consensus")?;
        let data = result.financial_data.unwrap_or_default();
        // Yahoo reports the analyst count as a JSON float; round to the nearest
        // whole number before narrowing to the domain's `u32`.
        let analysts = data
            .number_of_analyst_opinions
            .value()
            .map(|value| value.round())
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| value as u32);
        Ok(vec![Estimate {
            symbol: query.symbol.clone(),
            kind: "consensus".to_string(),
            fiscal_period: None,
            date: None,
            analyst: None,
            recommendation: data.recommendation_key,
            value: data.target_mean_price.value(),
            low: data.target_low_price.value(),
            high: data.target_high_price.value(),
            mean: data
                .target_mean_price
                .value()
                .or_else(|| data.current_price.value()),
            number_of_analysts: analysts,
            currency: data.financial_currency,
        }])
    }
}

// ---------------------------------------------------------------------------
// YahooHttpFuturesHistoricalFetcher — derivatives/futures/historical →
// EquityHistoricalData (reuses the v8 chart shape; futures symbol e.g. `ES=F`)
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo futures-historical fetcher (`v8 chart`, futures contract symbol).
    pub YahooHttpFuturesHistoricalFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<YahooSymbolQuery, EquityHistoricalData> for YahooHttpFuturesHistoricalFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "futures_historical";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v8/finance/chart/{}?interval=1d&range=1mo",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo futures_historical")?;
        yahoo_get(&client, &url, "yahoo futures_historical").await
    }

    fn transform_data(
        &self,
        query: &YahooSymbolQuery,
        raw: Bytes,
    ) -> Result<Vec<EquityHistoricalData>> {
        decode_chart_bars(&raw, &query.symbol, "yahoo futures_historical")
    }
}

/// Decode a v8 chart envelope into OHLCV rows, dropping all-null bars. Shared
/// by the equity-historical and futures-historical fetchers.
fn decode_chart_bars(raw: &Bytes, symbol: &str, ctx: &str) -> Result<Vec<EquityHistoricalData>> {
    let envelope: ChartEnvelope = serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("{ctx} parse_json: {error}")))?;
    if let Some(error) = envelope.chart.error {
        return Err(Error::Provider(format!("{ctx} chart error: {error}")));
    }
    let Some(series) = envelope.chart.result.into_iter().next() else {
        return Ok(Vec::new());
    };
    let Some(quote) = series.indicators.quote.into_iter().next() else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::with_capacity(series.timestamp.len());
    for (idx, timestamp) in series.timestamp.iter().enumerate() {
        let (Some(open), Some(high), Some(low), Some(close), Some(volume)) = (
            quote.open.get(idx).copied().flatten(),
            quote.high.get(idx).copied().flatten(),
            quote.low.get(idx).copied().flatten(),
            quote.close.get(idx).copied().flatten(),
            quote.volume.get(idx).copied().flatten(),
        ) else {
            continue;
        };
        rows.push(EquityHistoricalData {
            symbol: symbol.to_string(),
            date: unix_to_iso_date(*timestamp),
            open,
            high,
            low,
            close,
            volume: u64::try_from(volume).unwrap_or(0),
        });
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------
// YahooHttpFuturesCurveFetcher — derivatives/futures/curve → FuturesCurvePoint
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo futures-curve fetcher (`v7 quote` over a root's contract chain).
    pub YahooHttpFuturesCurveFetcher,
    BASE_URL
);

#[derive(Deserialize)]
struct CurveQuoteRow {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "regularMarketPrice", default)]
    regular_market_price: Option<f64>,
    #[serde(rename = "expireDate", default)]
    expire_date: Option<i64>,
    #[serde(rename = "underlyingSymbol", default)]
    underlying_symbol: Option<String>,
}

#[async_trait]
impl Fetcher<YahooSymbolQuery, FuturesCurvePoint> for YahooHttpFuturesCurveFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "futures_curve";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        // Yahoo's quote endpoint returns the front contract plus its
        // `futuresChain` when queried for a continuous root (e.g. `ES=F`).
        let url = format!(
            "{}/v7/finance/quote?symbols={}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo futures_curve")?;
        yahoo_get(&client, &url, "yahoo futures_curve").await
    }

    fn transform_data(
        &self,
        query: &YahooSymbolQuery,
        raw: Bytes,
    ) -> Result<Vec<FuturesCurvePoint>> {
        // The curve uses the same v7 quote envelope; each result row is one
        // contract along the forward curve.
        #[derive(Deserialize)]
        struct CurveEnvelope {
            #[serde(rename = "quoteResponse")]
            quote_response: CurveBody,
        }
        #[derive(Deserialize)]
        struct CurveBody {
            #[serde(default)]
            result: Vec<CurveQuoteRow>,
            #[serde(default)]
            error: Option<Value>,
        }
        let envelope: CurveEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("yahoo futures_curve parse_json: {error}")))?;
        if let Some(error) = envelope.quote_response.error {
            return Err(Error::Provider(format!(
                "yahoo futures_curve error: {error}"
            )));
        }
        let rows = envelope
            .quote_response
            .result
            .into_iter()
            .map(|row| {
                let contract_symbol = row.symbol.unwrap_or_else(|| query.symbol.clone());
                FuturesCurvePoint {
                    underlying: row
                        .underlying_symbol
                        .unwrap_or_else(|| query.symbol.clone()),
                    contract_symbol,
                    price: row.regular_market_price,
                    expiration: row.expire_date.map(unix_to_iso_date),
                }
            })
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// YahooHttpOptionsChainFetcher — derivatives/options/chains → OptionContract
// ---------------------------------------------------------------------------

tdw_core::provider_fetcher_struct!(
    /// Yahoo options-chain fetcher (`v7 options`).
    pub YahooHttpOptionsChainFetcher,
    BASE_URL
);

#[derive(Deserialize)]
struct OptionsEnvelope {
    #[serde(rename = "optionChain")]
    option_chain: OptionsBody,
}

#[derive(Deserialize)]
struct OptionsBody {
    #[serde(default)]
    result: Vec<OptionsResult>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct OptionsResult {
    #[serde(rename = "underlyingSymbol", default)]
    underlying_symbol: Option<String>,
    #[serde(default)]
    options: Vec<OptionsExpiry>,
}

#[derive(Deserialize)]
struct OptionsExpiry {
    #[serde(rename = "expirationDate", default)]
    expiration_date: i64,
    #[serde(default)]
    calls: Vec<OptionRow>,
    #[serde(default)]
    puts: Vec<OptionRow>,
}

#[derive(Deserialize)]
struct OptionRow {
    #[serde(rename = "contractSymbol", default)]
    contract_symbol: Option<String>,
    #[serde(default)]
    strike: f64,
    #[serde(default)]
    bid: Option<f64>,
    #[serde(default)]
    ask: Option<f64>,
    #[serde(rename = "lastPrice", default)]
    last_price: Option<f64>,
    #[serde(default)]
    volume: Option<u64>,
    #[serde(rename = "openInterest", default)]
    open_interest: Option<u64>,
    #[serde(rename = "impliedVolatility", default)]
    implied_volatility: Option<f64>,
}

#[async_trait]
impl Fetcher<YahooSymbolQuery, OptionContract> for YahooHttpOptionsChainFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "options_chains";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v7/finance/options/{}",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo options")?;
        yahoo_get(&client, &url, "yahoo options").await
    }

    fn transform_data(&self, query: &YahooSymbolQuery, raw: Bytes) -> Result<Vec<OptionContract>> {
        let envelope: OptionsEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("yahoo options parse_json: {error}")))?;
        if let Some(error) = envelope.option_chain.error {
            return Err(Error::Provider(format!("yahoo options error: {error}")));
        }
        let Some(result) = envelope.option_chain.result.into_iter().next() else {
            return Ok(Vec::new());
        };
        let underlying = result
            .underlying_symbol
            .unwrap_or_else(|| query.symbol.clone());
        let mut rows = Vec::new();
        for expiry in result.options {
            let expiration = unix_to_iso_date(expiry.expiration_date);
            for (option_type, side) in [("call", expiry.calls), ("put", expiry.puts)] {
                for opt in side {
                    rows.push(OptionContract {
                        underlying_symbol: underlying.clone(),
                        contract_symbol: opt.contract_symbol,
                        expiration: expiration.clone(),
                        strike: opt.strike,
                        option_type: option_type.to_string(),
                        bid: opt.bid,
                        ask: opt.ask,
                        last_price: opt.last_price,
                        volume: opt.volume,
                        open_interest: opt.open_interest,
                        implied_volatility: opt.implied_volatility,
                        delta: None,
                        gamma: None,
                        theta: None,
                        vega: None,
                        rho: None,
                    });
                }
            }
        }
        Ok(rows)
    }
}

// ===========================================================================
// openbb-parity P4W3: yfinance discovery screeners + ETF info.
//
// Discovery screeners are served by Yahoo's keyless predefined-screener API
// (`/v1/finance/screener/predefined/saved?scrIds=<ID>&count=N`), normalized to
// `tdw_domain::ScreenerRow`. One shared fetcher serves every predefined screen;
// the screen id is injected per dispatch binding (the FMP-discovery pattern).
//
// ETF info is served by the v10 `quoteSummary` `assetProfile`+`fundProfile`
// +`price` modules, normalized to `tdw_domain::EtfInfo`. ETF historical reuses
// the equity-historical fetcher (an ETF ticker resolves through the v8 chart
// endpoint like any symbol), so no dedicated fetcher is added here.
// ===========================================================================

/// Quote row returned by the Yahoo predefined-screener API. Each predefined
/// screen returns a `quotes` array of these (a superset of the v7 quote row).
#[derive(Deserialize)]
struct ScreenerQuoteRow {
    #[serde(default)]
    symbol: Option<String>,
    #[serde(rename = "longName", default)]
    long_name: Option<String>,
    #[serde(rename = "shortName", default)]
    short_name: Option<String>,
    #[serde(rename = "regularMarketPrice", default)]
    regular_market_price: RawNum,
    #[serde(rename = "regularMarketVolume", default)]
    regular_market_volume: RawNum,
    #[serde(rename = "marketCap", default)]
    market_cap: RawNum,
    #[serde(default)]
    sector: Option<String>,
    #[serde(default)]
    industry: Option<String>,
    #[serde(rename = "fullExchangeName", default)]
    full_exchange_name: Option<String>,
    #[serde(rename = "quoteType", default)]
    quote_type: Option<String>,
    #[serde(rename = "beta", default)]
    beta: RawNum,
}

#[derive(Deserialize)]
struct ScreenerEnvelope {
    finance: ScreenerFinance,
}

#[derive(Deserialize)]
struct ScreenerFinance {
    #[serde(default)]
    result: Vec<ScreenerResult>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct ScreenerResult {
    #[serde(default)]
    quotes: Vec<ScreenerQuoteRow>,
}

tdw_core::provider_fetcher_struct!(
    /// Yahoo predefined-screener fetcher (`v1 screener/predefined/saved`).
    ///
    /// Serves the keyless discovery screens (`aggressive_small_caps`,
    /// `growth_technology_stocks`, `undervalued_growth_stocks`,
    /// `undervalued_large_caps`); the screen id is carried in the query's
    /// `scr_ids` field, injected per dispatch binding.
    pub YahooHttpPredefinedScreenerFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<crate::YahooScreenerQuery, tdw_domain::ScreenerRow>
    for YahooHttpPredefinedScreenerFetcher
{
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "predefined_screener";

    fn transform_query(params: Value) -> Result<crate::YahooScreenerQuery> {
        crate::YahooScreenerQuery::from_value(&params)
    }

    async fn extract_data(
        &self,
        query: &crate::YahooScreenerQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        let url = format!(
            "{}/v1/finance/screener/predefined/saved?scrIds={}&count={}",
            self.base_url().trim_end_matches('/'),
            query.scr_ids,
            query.count,
        );
        let client = yahoo_client("yahoo predefined_screener")?;
        yahoo_get(&client, &url, "yahoo predefined_screener").await
    }

    fn transform_data(
        &self,
        _query: &crate::YahooScreenerQuery,
        raw: Bytes,
    ) -> Result<Vec<tdw_domain::ScreenerRow>> {
        let envelope: ScreenerEnvelope = serde_json::from_slice(&raw).map_err(|error| {
            Error::Provider(format!("yahoo predefined_screener parse_json: {error}"))
        })?;
        if let Some(error) = envelope.finance.error {
            return Err(Error::Provider(format!(
                "yahoo predefined_screener error: {error}"
            )));
        }
        let Some(result) = envelope.finance.result.into_iter().next() else {
            return Ok(Vec::new());
        };
        let rows = result
            .quotes
            .into_iter()
            .filter_map(|quote| {
                let symbol = quote.symbol.filter(|s| !s.trim().is_empty())?;
                Some(tdw_domain::ScreenerRow {
                    symbol,
                    company_name: quote.long_name.or(quote.short_name),
                    market_cap: quote.market_cap.value(),
                    sector: quote.sector,
                    industry: quote.industry,
                    beta: quote.beta.value(),
                    price: quote.regular_market_price.value(),
                    last_annual_dividend: None,
                    volume: quote.regular_market_volume.value(),
                    exchange: quote.full_exchange_name,
                    exchange_short_name: None,
                    country: None,
                    is_etf: Some(
                        quote
                            .quote_type
                            .as_deref()
                            .is_some_and(|t| t.eq_ignore_ascii_case("ETF")),
                    ),
                    is_actively_trading: None,
                })
            })
            .collect();
        Ok(rows)
    }
}

tdw_core::provider_fetcher_struct!(
    /// Yahoo ETF-info fetcher (`v10 quoteSummary` `price`+`fundProfile`
    /// +`defaultKeyStatistics`).
    pub YahooHttpEtfInfoFetcher,
    BASE_URL
);

#[derive(Deserialize, Default)]
struct FundProfile {
    #[serde(rename = "family", default)]
    family: Option<String>,
    #[serde(rename = "legalType", default)]
    legal_type: Option<String>,
    #[serde(rename = "feesExpensesInvestment", default)]
    fees_expenses_investment: Option<FundFees>,
}

#[derive(Deserialize, Default)]
struct FundFees {
    #[serde(rename = "annualReportExpenseRatio", default)]
    annual_report_expense_ratio: RawNum,
}

#[async_trait]
impl Fetcher<YahooSymbolQuery, tdw_domain::EtfInfo> for YahooHttpEtfInfoFetcher {
    const PROVIDER: &'static str = "yahoo";
    const ENDPOINT: &'static str = "etf_info";

    fn transform_query(params: Value) -> Result<YahooSymbolQuery> {
        YahooSymbolQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &YahooSymbolQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/v10/finance/quoteSummary/{}?modules=price,fundProfile,defaultKeyStatistics",
            self.base_url().trim_end_matches('/'),
            query.symbol,
        );
        let client = yahoo_client("yahoo etf_info")?;
        yahoo_get(&client, &url, "yahoo etf_info").await
    }

    fn transform_data(
        &self,
        query: &YahooSymbolQuery,
        raw: Bytes,
    ) -> Result<Vec<tdw_domain::EtfInfo>> {
        // `fundProfile` rides alongside the shared quoteSummary modules; decode it
        // with a local extension of the result block.
        #[derive(Deserialize, Default)]
        struct EtfResult {
            #[serde(default)]
            price: Option<PriceModule>,
            #[serde(rename = "fundProfile", default)]
            fund_profile: Option<FundProfile>,
        }
        #[derive(Deserialize)]
        struct EtfBody {
            #[serde(default)]
            result: Vec<EtfResult>,
            #[serde(default)]
            error: Option<Value>,
        }
        #[derive(Deserialize)]
        struct EtfEnvelope {
            #[serde(rename = "quoteSummary")]
            quote_summary: EtfBody,
        }
        let envelope: EtfEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("yahoo etf_info parse_json: {error}")))?;
        if let Some(error) = envelope.quote_summary.error {
            return Err(Error::Provider(format!("yahoo etf_info error: {error}")));
        }
        let Some(result) = envelope.quote_summary.result.into_iter().next() else {
            return Ok(Vec::new());
        };
        let price = result.price.unwrap_or_default();
        let fund = result.fund_profile.unwrap_or_default();
        let name = price
            .long_name
            .or(price.short_name)
            .unwrap_or_else(|| query.symbol.clone());
        Ok(vec![tdw_domain::EtfInfo {
            symbol: query.symbol.clone(),
            name,
            issuer: fund.family,
            nav: None,
            aum: None,
            expense_ratio: fund
                .fees_expenses_investment
                .and_then(|f| f.annual_report_expense_ratio.value()),
            holdings_count: None,
            currency: price.currency,
            exchange: price.exchange_name,
            inception_date: None,
            description: fund.legal_type,
        }])
    }
}

/// Convert a Unix timestamp (seconds since 1970-01-01 UTC) to a
/// `YYYY-MM-DD` calendar-date string in UTC. Uses Howard Hinnant's
/// civil_from_days algorithm; correct for all Gregorian dates.
fn unix_to_iso_date(timestamp_seconds: i64) -> String {
    tdw_core::date::unix_seconds_to_iso_date(timestamp_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_chart_url_uses_range_without_dates() {
        let url = build_chart_url("https://x", "AAPL", "1d", "5d", None, None, 1_700_000_000);
        assert_eq!(url, "https://x/v8/finance/chart/AAPL?interval=1d&range=5d");
    }

    #[test]
    fn build_chart_url_honors_date_window() {
        let start = tdw_core::Date::parse("2024-01-02").expect("date");
        let end = tdw_core::Date::parse("2024-01-31").expect("date");
        let url = build_chart_url(
            "https://x",
            "AAPL",
            "1d",
            "5d",
            Some(start),
            Some(end),
            9_999,
        );
        // 2024-01-02 -> 1_704_153_600; 2024-01-31 -> 1_706_659_200, and period2
        // is exclusive so the end date is pushed forward one day (+86_400).
        assert_eq!(
            url,
            "https://x/v8/finance/chart/AAPL?interval=1d&period1=1704153600&period2=1706745600"
        );
    }

    #[test]
    fn build_chart_url_open_ended_windows() {
        let date = tdw_core::Date::parse("2024-01-02").expect("date");
        // Only an end date: the window runs from the epoch (Yahoo clamps to the
        // listing date) to the exclusive end.
        let end_only = build_chart_url("https://x", "AAPL", "1d", "5d", None, Some(date), 9_999);
        assert_eq!(
            end_only,
            "https://x/v8/finance/chart/AAPL?interval=1d&period1=0&period2=1704240000"
        );
        // Only a start date: the window runs to `now`.
        let start_only = build_chart_url("https://x", "AAPL", "1d", "5d", Some(date), None, 9_999);
        assert_eq!(
            start_only,
            "https://x/v8/finance/chart/AAPL?interval=1d&period1=1704153600&period2=9999"
        );
    }

    #[test]
    fn unix_to_iso_date_matches_well_known_dates() {
        assert_eq!(unix_to_iso_date(0), "1970-01-01");
        assert_eq!(unix_to_iso_date(1_700_000_000), "2023-11-14");
        // Leap day check.
        assert_eq!(unix_to_iso_date(1_582_934_400), "2020-02-29");
        // Pre-epoch sanity check.
        assert_eq!(unix_to_iso_date(-86_400), "1969-12-31");
    }
}
