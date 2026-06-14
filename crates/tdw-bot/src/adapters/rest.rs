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
    /// # Errors
    ///
    /// Returns a [`DataError`] if the underlying HTTP client cannot be built.
    pub fn new(base_url: impl Into<String>) -> Result<Self, DataError> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .map_err(|error| DataError::new(format!("http client: {error}")))?;
        Ok(Self {
            base_url: base_url.into(),
            client,
        })
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
