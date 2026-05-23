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

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, RegistryEntry, Result};
use tdw_domain::EquityHistoricalData;
use tdw_provider_fileset::EquityHistoricalQuery;

const DEFAULT_INTERVAL: &str = "1d";
const DEFAULT_RANGE: &str = "5d";
const USER_AGENT: &str = "tdw-provider-yahoo/0.1";

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
        let url = format!(
            "{}/v8/finance/chart/{}?interval={}&range={}",
            self.base_url, query.symbol, self.interval, self.range
        );
        let client = Client::builder()
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
        let envelope: ChartEnvelope = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("yahoo parse_json: {error}")))?;
        if let Some(error) = envelope.chart.error {
            return Err(Error::Provider(format!("yahoo chart error: {error}")));
        }
        let series = envelope
            .chart
            .result
            .into_iter()
            .next()
            .ok_or_else(|| Error::Provider("yahoo response missing result[0]".to_string()))?;
        let quote = series
            .indicators
            .quote
            .into_iter()
            .next()
            .ok_or_else(|| Error::Provider("yahoo response missing quote[0]".to_string()))?;

        let mut rows = Vec::with_capacity(series.timestamp.len());
        for (idx, timestamp) in series.timestamp.iter().enumerate() {
            // Drop bars where Yahoo emitted nulls (Yahoo sometimes
            // includes a "current" bar with all-null fields when the
            // requested range overlaps an open session).
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
                symbol: query.symbol.clone(),
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
}

/// Convert a Unix timestamp (seconds since 1970-01-01 UTC) to a
/// `YYYY-MM-DD` calendar-date string in UTC. Uses Howard Hinnant's
/// civil_from_days algorithm; correct for all Gregorian dates.
fn unix_to_iso_date(timestamp_seconds: i64) -> String {
    let days_since_epoch = timestamp_seconds.div_euclid(86_400);
    // Shift so the era starts at 0000-03-01.
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
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
