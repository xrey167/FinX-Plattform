//! The production [`DataSource`]: a thin REST client over the warehouse data API.
//!
//! The warehouse app server exposes `GET /api/v1/{route...}?<params>`, returning
//! a `ResultEnvelope`-shaped JSON body whose `results` array carries the
//! standardized rows. This client maps each chat command to its catalog route
//! and tolerantly projects the returned rows into the bot's [`Quote`] /
//! [`NewsItem`] / [`MarketDataBar`] shapes. It holds no business logic beyond
//! that mapping — the router owns command dispatch and formatting.

use serde_json::Value;
use tdw_domain::{MarketDataBar, TimeGranularity};

use crate::data::{DataError, DataSource, NewsItem, Quote};

/// A thin blocking REST client over the warehouse `GET /api/v1/{route}` surface.
pub struct RestDataSource {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl RestDataSource {
    /// Build a client against a warehouse base URL (e.g. `http://127.0.0.1:7878`).
    ///
    /// The `base_url` is validated as an absolute URL (it must carry a scheme and
    /// host) and the underlying HTTP client is given a 10-second request timeout
    /// so a stalled warehouse cannot hang a chat reply indefinitely.
    ///
    /// # Errors
    ///
    /// Returns a [`DataError`] if `base_url` is not a valid absolute URL, or if
    /// the underlying HTTP client cannot be built.
    pub fn new(base_url: impl Into<String>) -> Result<Self, DataError> {
        let base_url = base_url.into();
        let parsed = reqwest::Url::parse(&base_url)
            .map_err(|error| DataError::new(format!("invalid base URL `{base_url}`: {error}")))?;
        if parsed.cannot_be_a_base() || !parsed.has_host() {
            return Err(DataError::new(format!(
                "invalid base URL `{base_url}`: a scheme and host are required"
            )));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|error| DataError::new(format!("http client: {error}")))?;
        Ok(Self { base_url, client })
    }

    /// Fetch a catalog route with a single `symbol` query param and return the
    /// envelope's `results` array.
    fn fetch_results(&self, route: &str, symbol: &str) -> Result<Vec<Value>, DataError> {
        let url = format!("{}/api/v1/{route}", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(&url)
            .query(&[("symbol", symbol)])
            .send()
            .map_err(|error| DataError::new(format!("request to {route} failed: {error}")))?;
        if !response.status().is_success() {
            return Err(DataError::new(format!(
                "{route} returned status {}",
                response.status()
            )));
        }
        let body: Value = response
            .json()
            .map_err(|error| DataError::new(format!("decoding {route}: {error}")))?;
        let results = body
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| DataError::new(format!("{route} response had no results array")))?;
        Ok(results)
    }
}

/// Read an `f64` from a row by trying each candidate key in order.
fn pick_f64(row: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| row.get(*key).and_then(Value::as_f64))
}

/// Read a `String` from a row by trying each candidate key in order.
fn pick_str(row: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| row.get(*key).and_then(Value::as_str))
        .map(ToString::to_string)
}

impl DataSource for RestDataSource {
    fn quote(&self, ticker: &str) -> Result<Quote, DataError> {
        let results = self.fetch_results("equity/price/quote", ticker)?;
        let row = results
            .first()
            .ok_or_else(|| DataError::new(format!("no quote row for {ticker}")))?;
        let price = pick_f64(row, &["last_price", "price", "close"])
            .ok_or_else(|| DataError::new(format!("quote for {ticker} had no price")))?;
        Ok(Quote {
            symbol: pick_str(row, &["symbol"]).unwrap_or_else(|| ticker.to_string()),
            price,
            change: pick_f64(row, &["change"]),
            change_percent: pick_f64(row, &["change_percent", "change_pct"]),
            currency: pick_str(row, &["currency"]),
        })
    }

    fn news(&self, ticker: &str) -> Result<Vec<NewsItem>, DataError> {
        let results = self.fetch_results("news/company", ticker)?;
        Ok(results
            .iter()
            .filter_map(|row| {
                Some(NewsItem {
                    title: pick_str(row, &["title", "headline"])?,
                    source: pick_str(row, &["source", "provider"]).unwrap_or_default(),
                    published: pick_str(row, &["date", "published", "published_at"])
                        .unwrap_or_default(),
                })
            })
            .collect())
    }

    fn history(&self, ticker: &str) -> Result<Vec<MarketDataBar>, DataError> {
        let results = self.fetch_results("equity/price/historical", ticker)?;
        Ok(results
            .iter()
            .filter_map(|row| {
                let close = pick_f64(row, &["close"])?;
                Some(MarketDataBar {
                    symbol: ticker.to_string(),
                    venue: pick_str(row, &["venue"]).unwrap_or_default(),
                    granularity: TimeGranularity::Day,
                    ts: pick_str(row, &["date", "ts"]).unwrap_or_default(),
                    open: pick_f64(row, &["open"]).unwrap_or(close),
                    high: pick_f64(row, &["high"]).unwrap_or(close),
                    low: pick_f64(row, &["low"]).unwrap_or(close),
                    close,
                    volume: pick_f64(row, &["volume"]).unwrap_or_default(),
                    source: "rest".to_string(),
                })
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::RestDataSource;

    #[test]
    fn new_rejects_a_schemeless_url() {
        // A bare host with no scheme is not an absolute URL and must be rejected
        // rather than silently producing a client that can never fetch.
        let Err(err) = RestDataSource::new("127.0.0.1:7878") else {
            panic!("schemeless URL must be rejected");
        };
        assert!(err.message.contains("invalid base URL"), "{}", err.message);
    }

    #[test]
    fn new_rejects_garbage() {
        let Err(err) = RestDataSource::new("not a url") else {
            panic!("garbage must be rejected");
        };
        assert!(err.message.contains("invalid base URL"), "{}", err.message);
    }

    #[test]
    fn new_accepts_a_valid_http_url() {
        assert!(RestDataSource::new("http://127.0.0.1:7878").is_ok());
    }
}
