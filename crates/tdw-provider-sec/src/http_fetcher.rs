//! Real SEC EDGAR HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public EDGAR data API
//! (`https://data.sec.gov`) without authentication. Live calls are gated by
//! `TDW_SEC_LIVE=1` in the integration test.
//!
//! SEC EDGAR has a soft rate limit of ~10 requests/second. A 100 ms sleep is
//! inserted after every HTTP request to stay well within that bound.

#![cfg(feature = "http")]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tdw_core::http_support::prelude::*;
use tdw_domain::{MarketDataBar, TimeGranularity};
use tokio::time::sleep;

use crate::{BASE_URL, SecFilingsQuery, SecHistoricalQuery};

const USER_AGENT: &str = "tdw-provider-sec/0.1 (contact@finx.example)";
/// SEC EDGAR enforces a soft 10 req/sec ceiling; we sleep 100 ms after each
/// request to stay comfortably below it.
const RATE_LIMIT_DELAY: Duration = Duration::from_millis(100);

// ── Filings fetcher ───────────────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC EDGAR submissions/filings fetcher.
    ///
    /// Calls `GET /submissions/CIK{cik_padded_10digits}.json`.
    pub SecFilingsHttpFetcher,
    BASE_URL
);

/// Wire shape returned by `GET /submissions/CIK*.json`.
#[derive(Deserialize)]
struct SecSubmissionsEnvelope {
    #[serde(default)]
    cik: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    filings: Option<SecFilingsWrapper>,
}

#[derive(Deserialize)]
struct SecFilingsWrapper {
    #[serde(default)]
    recent: Option<SecRecentFilings>,
}

#[derive(Deserialize)]
struct SecRecentFilings {
    #[serde(rename = "accessionNumber", default)]
    accession_number: Vec<String>,
    #[serde(default)]
    form: Vec<String>,
    #[serde(rename = "filingDate", default)]
    filing_date: Vec<String>,
}

/// Flattened row returned by `transform_data` for filings queries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SecFiling {
    pub cik: String,
    pub entity_name: String,
    pub accession_number: String,
    pub form: String,
    pub filing_date: String,
}

#[async_trait]
impl Fetcher<SecFilingsQuery, SecFiling> for SecFilingsHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "filings";

    fn transform_query(params: Value) -> Result<SecFilingsQuery> {
        let cik = params
            .get("cik")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("sec cik must be a string".to_string()))?;
        SecFilingsQuery::new(cik).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &SecFilingsQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/submissions/CIK{}.json",
            self.base_url().trim_end_matches('/'),
            query.padded_cik(),
        );

        let client = tdw_core::http_support::build_client(USER_AGENT, "sec http client build")?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("sec filings extract_data: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec filings returned {status}: {body}"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec filings read body: {e}")))?;

        // Rate-limit: wait after each successful request.
        sleep(RATE_LIMIT_DELAY).await;

        Ok(bytes)
    }

    fn transform_data(&self, query: &SecFilingsQuery, raw: Bytes) -> Result<Vec<SecFiling>> {
        let envelope: SecSubmissionsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("sec filings parse_json: {e}")))?;

        let cik = envelope.cik.unwrap_or_else(|| query.cik.clone());
        let entity_name = envelope.name.unwrap_or_default();

        let recent = envelope
            .filings
            .and_then(|f| f.recent)
            .unwrap_or(SecRecentFilings {
                accession_number: Vec::new(),
                form: Vec::new(),
                filing_date: Vec::new(),
            });

        let len = recent.accession_number.len();
        let mut rows = Vec::with_capacity(len);

        for i in 0..len {
            let accession_number = recent.accession_number.get(i).cloned().unwrap_or_default();
            let form = recent.form.get(i).cloned().unwrap_or_default();
            let filing_date = recent.filing_date.get(i).cloned().unwrap_or_default();

            rows.push(SecFiling {
                cik: cik.clone(),
                entity_name: entity_name.clone(),
                accession_number,
                form,
                filing_date,
            });
        }

        Ok(rows)
    }
}

// ── XBRL company-facts fetcher ────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC EDGAR XBRL company-facts fetcher.
    ///
    /// Calls `GET /api/xbrl/companyfacts/CIK{cik_padded_10digits}.json`.
    /// Returns equity historical data shaped as [`MarketDataBar`] by extracting
    /// `us-gaap/Revenue` USD facts tagged on 10-K filings.
    pub SecXbrlHttpFetcher,
    BASE_URL
);

/// Wire shape for `GET /api/xbrl/companyfacts/CIK*.json`.
#[derive(Deserialize)]
struct SecXbrlEnvelope {
    #[allow(dead_code)]
    #[serde(default)]
    cik: Option<u64>,
    #[serde(rename = "entityName", default)]
    entity_name: Option<String>,
    #[serde(default)]
    facts: Option<SecXbrlFacts>,
}

#[derive(Deserialize)]
struct SecXbrlFacts {
    #[serde(rename = "us-gaap", default)]
    us_gaap: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
struct SecXbrlConcept {
    #[allow(dead_code)]
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    units: Option<SecXbrlUnits>,
}

#[derive(Deserialize)]
struct SecXbrlUnits {
    #[serde(rename = "USD", default)]
    usd: Vec<SecXbrlFact>,
}

#[derive(Deserialize)]
struct SecXbrlFact {
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    val: Option<f64>,
    #[serde(default)]
    form: Option<String>,
}

#[async_trait]
impl Fetcher<SecHistoricalQuery, MarketDataBar> for SecXbrlHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "xbrl_revenue";

    fn transform_query(params: Value) -> Result<SecHistoricalQuery> {
        let symbol = params
            .get("symbol")
            .or_else(|| params.get("ticker"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("sec symbol must be a string".to_string()))?;
        SecHistoricalQuery::new(symbol).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(
        &self,
        query: &SecHistoricalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        // SEC EDGAR XBRL endpoint requires a CIK, not a ticker symbol. The
        // caller is expected to pass the CIK as the `symbol` field (numeric
        // string). Validate that the symbol looks like a numeric CIK.
        let cik_query = SecFilingsQuery::new(&query.symbol).map_err(|_| {
            Error::InvalidQuery(format!(
                "sec xbrl_revenue expects a numeric CIK in `symbol`, got: {}",
                query.symbol
            ))
        })?;

        let url = format!(
            "{}/api/xbrl/companyfacts/CIK{}.json",
            self.base_url().trim_end_matches('/'),
            cik_query.padded_cik(),
        );

        let client = tdw_core::http_support::build_client(USER_AGENT, "sec http client build")?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("sec xbrl extract_data: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec xbrl returned {status}: {body}"
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec xbrl read body: {e}")))?;

        // Rate-limit: wait after each successful request.
        sleep(RATE_LIMIT_DELAY).await;

        Ok(bytes)
    }

    fn transform_data(&self, query: &SecHistoricalQuery, raw: Bytes) -> Result<Vec<MarketDataBar>> {
        let envelope: SecXbrlEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("sec xbrl parse_json: {e}")))?;

        let entity_name = envelope.entity_name.unwrap_or_else(|| query.symbol.clone());

        let us_gaap = envelope.facts.and_then(|f| f.us_gaap).unwrap_or_default();

        let revenue_value = us_gaap.get("Revenue").cloned();
        let revenue: SecXbrlConcept = match revenue_value {
            Some(v) => serde_json::from_value(v)
                .map_err(|e| Error::Provider(format!("sec xbrl Revenue parse: {e}")))?,
            None => {
                return Ok(Vec::new());
            }
        };

        let usd_facts = revenue.units.map(|u| u.usd).unwrap_or_default();

        let mut rows = Vec::new();
        for fact in usd_facts {
            // Only include 10-K annual facts with a known end date and value.
            let is_annual = fact
                .form
                .as_deref()
                .map(|f| f.eq_ignore_ascii_case("10-K"))
                .unwrap_or(false);
            if !is_annual {
                continue;
            }
            let end_date = match fact.end {
                Some(d) => d,
                None => continue,
            };
            let val = match fact.val {
                Some(v) => v,
                None => continue,
            };

            rows.push(MarketDataBar {
                symbol: entity_name.clone(),
                venue: "sec".to_string(),
                granularity: TimeGranularity::Day,
                ts: format!("{end_date}T00:00:00Z"),
                open: val,
                high: val,
                low: val,
                close: val,
                volume: 0.0,
                source: "sec-xbrl".to_string(),
            });
        }

        Ok(rows)
    }
}

// ── Inline unit tests for transform_data (no network) ────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn filings_query() -> SecFilingsQuery {
        SecFilingsQuery::new("320193").expect("valid cik")
    }

    fn historical_query() -> SecHistoricalQuery {
        SecHistoricalQuery::new("320193").expect("valid cik-as-symbol")
    }

    #[test]
    fn transform_query_filings_accepts_valid_cik() {
        let q = SecFilingsHttpFetcher::transform_query(serde_json::json!({"cik": "320193"}))
            .expect("transform_query must succeed");
        assert_eq!(q.cik, "320193");
        assert_eq!(q.padded_cik(), "0000320193");
    }

    #[test]
    fn transform_query_filings_rejects_missing_cik() {
        assert!(SecFilingsHttpFetcher::transform_query(serde_json::json!({})).is_err());
    }

    #[test]
    fn transform_data_filings_parses_cassette() {
        let fetcher = SecFilingsHttpFetcher::default();
        let query = filings_query();
        let raw = Bytes::from(
            serde_json::json!({
                "cik": "320193",
                "name": "Apple Inc.",
                "filings": {
                    "recent": {
                        "accessionNumber": ["0000320193-24-000123"],
                        "form": ["10-K"],
                        "filingDate": ["2024-10-01"]
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let rows = fetcher
            .transform_data(&query, raw)
            .expect("transform_data must succeed");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cik, "320193");
        assert_eq!(rows[0].entity_name, "Apple Inc.");
        assert_eq!(rows[0].form, "10-K");
        assert_eq!(rows[0].filing_date, "2024-10-01");
    }

    #[test]
    fn transform_data_xbrl_parses_cassette() {
        let fetcher = SecXbrlHttpFetcher::default();
        let query = historical_query();
        let raw = Bytes::from(
            serde_json::json!({
                "cik": 320193,
                "entityName": "Apple Inc.",
                "facts": {
                    "us-gaap": {
                        "Revenue": {
                            "label": "Revenue",
                            "units": {
                                "USD": [
                                    {"end": "2024-09-28", "val": 391035000000.0_f64, "form": "10-K"},
                                    {"end": "2023-09-30", "val": 383285000000.0_f64, "form": "10-K"},
                                    {"end": "2024-03-30", "val": 90753000000.0_f64, "form": "10-Q"}
                                ]
                            }
                        }
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let rows = fetcher
            .transform_data(&query, raw)
            .expect("transform_data must succeed");
        // Only the two 10-K rows should appear; 10-Q is excluded.
        assert_eq!(rows.len(), 2, "rows={rows:#?}");
        assert_eq!(rows[0].ts, "2024-09-28T00:00:00Z");
        assert_eq!(rows[0].close, 391_035_000_000.0);
        assert_eq!(rows[0].source, "sec-xbrl");
        assert_eq!(rows[1].ts, "2023-09-30T00:00:00Z");
    }

    #[test]
    fn transform_data_xbrl_returns_empty_when_no_revenue_concept() {
        let fetcher = SecXbrlHttpFetcher::default();
        let query = historical_query();
        let raw = Bytes::from(
            serde_json::json!({
                "cik": 320193,
                "entityName": "Apple Inc.",
                "facts": {
                    "us-gaap": {}
                }
            })
            .to_string()
            .into_bytes(),
        );
        let rows = fetcher
            .transform_data(&query, raw)
            .expect("empty facts must not error");
        assert!(rows.is_empty());
    }

    #[test]
    fn transform_query_xbrl_normalises_symbol() {
        let q = SecXbrlHttpFetcher::transform_query(serde_json::json!({"symbol": "320193"}))
            .expect("transform_query must succeed");
        assert_eq!(q.symbol, "320193");
    }

    #[test]
    fn transform_query_xbrl_rejects_missing_symbol() {
        assert!(SecXbrlHttpFetcher::transform_query(serde_json::json!({})).is_err());
    }
}
