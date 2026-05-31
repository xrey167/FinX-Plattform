//! Real CoinGecko OHLC backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to CoinGecko's OHLC endpoint
//! directly via `reqwest`. The free Demo API tier requires no key, but
//! an optional `COINGECKO_API_KEY` is forwarded when present via the
//! `x-cg-demo-api-key` header. Live calls are additionally gated by
//! `TDW_COINGECKO_LIVE=1` so unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::{API_KEY_HEADER, BASE_URL, CoinGeckoOhlcQuery, ohlc_request};

const API_KEY_ENV: &str = "COINGECKO_API_KEY";
const USER_AGENT: &str = "tdw-provider-coingecko/0.1";

/// Production CoinGecko OHLC fetcher.
#[derive(Clone, Debug)]
pub struct CoinGeckoHttpOhlcFetcher {
    base_url: String,
}

impl Default for CoinGeckoHttpOhlcFetcher {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.to_string(),
        }
    }
}

impl CoinGeckoHttpOhlcFetcher {
    /// Override the CoinGecko base URL (useful for tests).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Registry entry advertised under the canonical `coingecko` provider name.
    pub fn registry_entry() -> RegistryEntry {
        RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
    }
}

#[async_trait]
impl Fetcher<CoinGeckoOhlcQuery, MarketDataBar> for CoinGeckoHttpOhlcFetcher {
    const PROVIDER: &'static str = "coingecko";
    const ENDPOINT: &'static str = "ohlc";

    fn transform_query(params: Value) -> Result<CoinGeckoOhlcQuery> {
        let coin_id = params
            .get("coin_id")
            .or_else(|| params.get("symbol"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("coingecko coin_id must be a string".to_string()))?;
        let vs_currency = params
            .get("vs_currency")
            .and_then(Value::as_str)
            .unwrap_or("usd");
        let days = params
            .get("days")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|error| Error::InvalidQuery(format!("coingecko days too large: {error}")))?
            .unwrap_or(30);

        CoinGeckoOhlcQuery::new(coin_id, vs_currency, days)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    async fn extract_data(
        &self,
        query: &CoinGeckoOhlcQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        ohlc_request(&query.coin_id, &query.vs_currency, query.days)
            .map_err(|error| Error::Provider(error.to_string()))?;

        let endpoint = format!(
            "{}/coins/{}/ohlc?vs_currency={}&days={}",
            self.base_url.trim_end_matches('/'),
            query.coin_id,
            query.vs_currency,
            query.days,
        );

        let mut request_builder = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("coingecko client: {error}")))?
            .get(&endpoint);

        if let Ok(api_key) = std::env::var(API_KEY_ENV) {
            let api_key = api_key.trim().to_string();
            if !api_key.is_empty() {
                request_builder = request_builder.header(API_KEY_HEADER, api_key);
            }
        }

        let response = request_builder
            .send()
            .await
            .map_err(|error| Error::Provider(format!("coingecko extract_data: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "coingecko extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("coingecko read body: {error}")))
    }

    fn transform_data(&self, query: &CoinGeckoOhlcQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        // CoinGecko OHLC response is a JSON array of arrays:
        // [[timestamp_ms, open, high, low, close], ...]
        let rows: Vec<[Value; 5]> = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("coingecko parse_json: {error}")))?;

        let mut bars = Vec::with_capacity(rows.len());
        for row in rows {
            let ts_ms = row[0]
                .as_i64()
                .ok_or_else(|| Error::Provider("coingecko: missing timestamp".to_string()))?;
            let open = row[1]
                .as_f64()
                .ok_or_else(|| Error::Provider("coingecko: missing open".to_string()))?;
            let high = row[2]
                .as_f64()
                .ok_or_else(|| Error::Provider("coingecko: missing high".to_string()))?;
            let low = row[3]
                .as_f64()
                .ok_or_else(|| Error::Provider("coingecko: missing low".to_string()))?;
            let close = row[4]
                .as_f64()
                .ok_or_else(|| Error::Provider("coingecko: missing close".to_string()))?;

            bars.push(MarketDataBar {
                symbol: query.coin_id.clone(),
                venue: "coingecko".to_string(),
                granularity: TimeGranularity::Day,
                ts: unix_millis_to_iso_timestamp(ts_ms),
                open,
                high,
                low,
                close,
                volume: 0.0,
                source: "coingecko".to_string(),
            });
        }
        Ok(bars)
    }
}

fn unix_millis_to_iso_timestamp(timestamp_millis: i64) -> String {
    let seconds = timestamp_millis.div_euclid(1_000);
    let days_since_epoch = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_millis_to_iso_timestamp_matches_well_known_dates() {
        assert_eq!(unix_millis_to_iso_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            unix_millis_to_iso_timestamp(1_704_153_600_000),
            "2024-01-02T00:00:00Z"
        );
        assert_eq!(
            unix_millis_to_iso_timestamp(-86_400_000),
            "1969-12-31T00:00:00Z"
        );
    }

    #[test]
    fn transform_query_defaults_vs_currency_and_days() {
        use serde_json::json;
        let query = CoinGeckoHttpOhlcFetcher::transform_query(json!({
            "coin_id": "bitcoin"
        }))
        .unwrap_or_else(|error| panic!("should parse: {error}"));
        assert_eq!(query.coin_id, "bitcoin");
        assert_eq!(query.vs_currency, "usd");
        assert_eq!(query.days, 30);
    }

    #[test]
    fn transform_query_accepts_symbol_alias() {
        use serde_json::json;
        let query = CoinGeckoHttpOhlcFetcher::transform_query(json!({
            "symbol": "ethereum",
            "vs_currency": "eur",
            "days": 7
        }))
        .unwrap_or_else(|error| panic!("should parse: {error}"));
        assert_eq!(query.coin_id, "ethereum");
        assert_eq!(query.vs_currency, "eur");
        assert_eq!(query.days, 7);
    }

    #[test]
    fn transform_data_parses_ohlc_array() {
        use bytes::Bytes;
        let fetcher = CoinGeckoHttpOhlcFetcher::default();
        let query = CoinGeckoOhlcQuery::new("bitcoin", "usd", 1)
            .unwrap_or_else(|error| panic!("query: {error}"));
        // CoinGecko OHLC format: [[ts_ms, open, high, low, close], ...]
        let raw = Bytes::from(r#"[[1704153600000, 42000.0, 43000.0, 41500.0, 42500.0]]"#);
        let bars = fetcher
            .transform_data(&query, raw)
            .unwrap_or_else(|error| panic!("transform_data: {error}"));
        assert_eq!(bars.len(), 1);
        let bar = &bars[0];
        assert_eq!(bar.symbol, "bitcoin");
        assert_eq!(bar.ts, "2024-01-02T00:00:00Z");
        assert!((bar.open - 42000.0).abs() < f64::EPSILON);
        assert!((bar.high - 43000.0).abs() < f64::EPSILON);
        assert!((bar.low - 41500.0).abs() < f64::EPSILON);
        assert!((bar.close - 42500.0).abs() < f64::EPSILON);
        assert_eq!(bar.volume, 0.0);
        assert_eq!(bar.venue, "coingecko");
    }
}
