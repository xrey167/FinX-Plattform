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
use tdw_domain::{
    CompanyFacts, CompanyFiling, EtfHolding, FilingFile, FilingHeader, LitigationRelease,
    MarketDataBar, OwnershipRecord, SecInstitution, SicCode, SymbolMapping, TimeGranularity,
};
use tokio::time::sleep;

use crate::{
    BASE_URL, SecAccessionQuery, SecCikMapQuery, SecFilingsQuery, SecHistoricalQuery,
    SecSearchQuery,
};

/// Public host for the keyless `company_tickers.json` ticker↔CIK directory.
///
/// The fails-to-deliver datasets also live here. The XBRL/submissions data lives
/// under [`crate::BASE_URL`] (`data.sec.gov`); the directory + static data files
/// live under `www.sec.gov`. Documented at
/// <https://www.sec.gov/search-filings/edgar-application-programming-interfaces>.
pub const WWW_BASE_URL: &str = "https://www.sec.gov";

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

        // Live EDGAR returns the CIK zero-padded to 10 digits; cassettes and
        // queries use the canonical unpadded form. Normalize so rows always
        // carry the unpadded CIK regardless of source.
        let cik = envelope.cik.map_or_else(
            || query.cik.clone(),
            |c| {
                let trimmed = c.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0".to_string()
                } else {
                    trimmed.to_string()
                }
            },
        );
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
        // Filers tag top-line revenue under different us-gaap concepts: modern
        // ASC-606 filers (e.g. Apple) use RevenueFromContractWithCustomer...,
        // older filings use Revenues/SalesRevenueNet, and some cassettes use
        // the bare "Revenue". Take the first concept present, in that order.
        const REVENUE_CONCEPTS: [&str; 4] = [
            "RevenueFromContractWithCustomerExcludingAssessedTax",
            "Revenues",
            "SalesRevenueNet",
            "Revenue",
        ];

        let envelope: SecXbrlEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("sec xbrl parse_json: {e}")))?;

        let entity_name = envelope.entity_name.unwrap_or_else(|| query.symbol.clone());

        let us_gaap = envelope.facts.and_then(|f| f.us_gaap).unwrap_or_default();

        let revenue_value = REVENUE_CONCEPTS
            .iter()
            .find_map(|concept| us_gaap.get(*concept).cloned());
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
                .is_some_and(|f| f.eq_ignore_ascii_case("10-K"));
            if !is_annual {
                continue;
            }
            let Some(end_date) = fact.end else { continue };
            let Some(val) = fact.val else { continue };

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

// ── XBRL company-facts fetcher → CompanyFacts (equity/compare/company_facts) ──

tdw_core::provider_fetcher_struct!(
    /// Production SEC EDGAR XBRL company-facts fetcher, normalized to
    /// [`tdw_domain::CompanyFacts`].
    ///
    /// Calls `GET /api/xbrl/companyfacts/CIK{cik_padded_10digits}.json` and
    /// flattens the `us-gaap` / `dei` taxonomy fact tree into one
    /// [`CompanyFacts`] row per (concept, unit, fact). Standardizes
    /// `equity/compare/company_facts`.
    pub SecCompanyFactsHttpFetcher,
    BASE_URL
);

/// Wire shape for `GET /api/xbrl/companyfacts/CIK*.json` (full taxonomy tree).
#[derive(Deserialize)]
struct SecCompanyFactsEnvelope {
    #[serde(default)]
    cik: Option<u64>,
    #[serde(rename = "entityName", default)]
    entity_name: Option<String>,
    #[serde(default)]
    facts: Option<
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, SecConceptFacts>>,
    >,
}

#[derive(Deserialize)]
struct SecConceptFacts {
    #[serde(default)]
    units: Option<std::collections::BTreeMap<String, Vec<SecCompanyFact>>>,
}

#[derive(Deserialize)]
struct SecCompanyFact {
    #[serde(default)]
    end: Option<String>,
    #[serde(default)]
    val: Option<f64>,
    #[serde(default)]
    fy: Option<i32>,
    #[serde(default)]
    fp: Option<String>,
    #[serde(default)]
    form: Option<String>,
}

#[async_trait]
impl Fetcher<SecFilingsQuery, CompanyFacts> for SecCompanyFactsHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "company_facts";

    fn transform_query(params: Value) -> Result<SecFilingsQuery> {
        let cik = params
            .get("cik")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("sec cik must be a string".to_string()))?;
        SecFilingsQuery::new(cik).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &SecFilingsQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/api/xbrl/companyfacts/CIK{}.json",
            self.base_url().trim_end_matches('/'),
            query.padded_cik(),
        );
        let client = tdw_core::http_support::build_client(USER_AGENT, "sec http client build")?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("sec company_facts extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec company_facts returned {status}: {body}"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec company_facts read body: {e}")))?;
        sleep(RATE_LIMIT_DELAY).await;
        Ok(bytes)
    }

    fn transform_data(&self, query: &SecFilingsQuery, raw: Bytes) -> Result<Vec<CompanyFacts>> {
        let envelope: SecCompanyFactsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("sec company_facts parse_json: {e}")))?;
        let cik = envelope
            .cik
            .map_or_else(|| query.cik.clone(), |c| c.to_string());
        let entity_name = envelope.entity_name;
        let mut rows = Vec::new();
        let Some(taxonomies) = envelope.facts else {
            return Ok(rows);
        };
        for (taxonomy, concepts) in taxonomies {
            for (concept, parsed) in concepts {
                let Some(units) = parsed.units else { continue };
                for (unit, facts) in units {
                    for fact in facts {
                        rows.push(CompanyFacts {
                            cik: cik.clone(),
                            entity_name: entity_name.clone(),
                            taxonomy: taxonomy.clone(),
                            concept: concept.clone(),
                            unit: Some(unit.clone()),
                            end: fact.end,
                            fiscal_year: fact.fy,
                            fiscal_period: fact.fp,
                            form: fact.form,
                            value: fact.val,
                        });
                    }
                }
            }
        }
        Ok(rows)
    }
}

// ── Latest financial reports fetcher (equity/discovery/latest_financial_reports) ─

tdw_core::provider_fetcher_struct!(
    /// Production SEC latest-financial-reports fetcher, normalized to
    /// [`tdw_domain::CompanyFiling`].
    ///
    /// Calls `GET /submissions/CIK{cik_padded_10digits}.json` and selects the
    /// periodic financial-report filings (10-K / 10-Q / 20-F / 40-F), emitting one
    /// [`CompanyFiling`] per report (the filer CIK is carried in `symbol`).
    /// Standardizes `equity/discovery/latest_financial_reports`.
    pub SecLatestFinancialReportsHttpFetcher,
    BASE_URL
);

/// SEC form types that carry periodic financial statements.
const FINANCIAL_REPORT_FORMS: &[&str] = &["10-K", "10-Q", "20-F", "40-F"];

#[async_trait]
impl Fetcher<SecFilingsQuery, CompanyFiling> for SecLatestFinancialReportsHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "latest_financial_reports";

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
        let response = client.get(&url).send().await.map_err(|e| {
            Error::Provider(format!("sec latest_financial_reports extract_data: {e}"))
        })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec latest_financial_reports returned {status}: {body}"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec latest_financial_reports read body: {e}")))?;
        sleep(RATE_LIMIT_DELAY).await;
        Ok(bytes)
    }

    fn transform_data(&self, query: &SecFilingsQuery, raw: Bytes) -> Result<Vec<CompanyFiling>> {
        let envelope: SecSubmissionsEnvelope = serde_json::from_slice(&raw).map_err(|e| {
            Error::Provider(format!("sec latest_financial_reports parse_json: {e}"))
        })?;
        let cik = envelope.cik.clone().map_or_else(
            || query.cik.clone(),
            |c| {
                let trimmed = c.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0".to_string()
                } else {
                    trimmed.to_string()
                }
            },
        );
        let recent = envelope
            .filings
            .and_then(|f| f.recent)
            .unwrap_or(SecRecentFilings {
                accession_number: Vec::new(),
                form: Vec::new(),
                filing_date: Vec::new(),
            });
        let len = recent.accession_number.len();
        let mut rows = Vec::new();
        for i in 0..len {
            let form = recent.form.get(i).cloned().unwrap_or_default();
            let is_financial = FINANCIAL_REPORT_FORMS
                .iter()
                .any(|f| form.eq_ignore_ascii_case(f));
            if !is_financial {
                continue;
            }
            rows.push(CompanyFiling {
                symbol: cik.clone(),
                form_type: form,
                filing_date: recent.filing_date.get(i).cloned(),
                accepted_date: None,
                cik: Some(cik.clone()),
                link: recent.accession_number.get(i).cloned(),
                final_link: None,
            });
        }
        Ok(rows)
    }
}

// ── N-PORT disclosure index fetcher (etf/nport_disclosure) ────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC N-PORT disclosure-index fetcher, normalized to
    /// [`tdw_domain::CompanyFiling`].
    ///
    /// Calls `GET /submissions/CIK{cik_padded_10digits}.json` and selects the fund
    /// portfolio-disclosure filings (`NPORT-P` / `NPORT-EX`), emitting one
    /// [`CompanyFiling`] per disclosure (the filer CIK is carried in `symbol`).
    /// This is the filing-level disclosure index — distinct from `etf/holdings`,
    /// which parses an individual N-PORT filing's portfolio XML into per-holding
    /// rows. Standardizes `etf/nport_disclosure`.
    pub SecNportDisclosureHttpFetcher,
    BASE_URL
);

/// SEC form types that carry fund portfolio (N-PORT) disclosures.
const NPORT_DISCLOSURE_FORMS: &[&str] = &["NPORT-P", "NPORT-EX"];

#[async_trait]
impl Fetcher<SecFilingsQuery, CompanyFiling> for SecNportDisclosureHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "nport_disclosure";

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
            .map_err(|e| Error::Provider(format!("sec nport_disclosure extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec nport_disclosure returned {status}: {body}"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec nport_disclosure read body: {e}")))?;
        sleep(RATE_LIMIT_DELAY).await;
        Ok(bytes)
    }

    fn transform_data(&self, query: &SecFilingsQuery, raw: Bytes) -> Result<Vec<CompanyFiling>> {
        let envelope: SecSubmissionsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("sec nport_disclosure parse_json: {e}")))?;
        let cik = envelope.cik.clone().map_or_else(
            || query.cik.clone(),
            |c| {
                let trimmed = c.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0".to_string()
                } else {
                    trimmed.to_string()
                }
            },
        );
        let recent = envelope
            .filings
            .and_then(|f| f.recent)
            .unwrap_or(SecRecentFilings {
                accession_number: Vec::new(),
                form: Vec::new(),
                filing_date: Vec::new(),
            });
        let len = recent.accession_number.len();
        let mut rows = Vec::new();
        for i in 0..len {
            let form = recent.form.get(i).cloned().unwrap_or_default();
            let is_nport = NPORT_DISCLOSURE_FORMS
                .iter()
                .any(|f| form.eq_ignore_ascii_case(f));
            if !is_nport {
                continue;
            }
            rows.push(CompanyFiling {
                symbol: cik.clone(),
                form_type: form,
                filing_date: recent.filing_date.get(i).cloned(),
                accepted_date: None,
                cik: Some(cik.clone()),
                link: recent.accession_number.get(i).cloned(),
                final_link: None,
            });
        }
        Ok(rows)
    }
}

// ── CIK ↔ ticker symbol map fetcher (regulators/sec/cik_map) ──────────────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC `company_tickers.json` ticker↔CIK map fetcher.
    ///
    /// Calls `GET https://www.sec.gov/files/company_tickers.json` (keyless,
    /// requires a User-Agent header). Standardizes `regulators/sec/cik_map` to
    /// [`tdw_domain::SymbolMapping`] rows.
    pub SecCikMapHttpFetcher,
    WWW_BASE_URL
);

/// Wire shape of one `company_tickers.json` entry. SEC keys the object by an
/// integer index; each value carries the CIK (as an integer), ticker, and title.
#[derive(Deserialize)]
struct SecCompanyTicker {
    #[serde(default)]
    cik_str: Option<u64>,
    #[serde(default)]
    ticker: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[async_trait]
impl Fetcher<SecCikMapQuery, SymbolMapping> for SecCikMapHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "cik_map";

    fn transform_query(_params: Value) -> Result<SecCikMapQuery> {
        Ok(SecCikMapQuery::new())
    }

    async fn extract_data(&self, _query: &SecCikMapQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/files/company_tickers.json",
            self.base_url().trim_end_matches('/'),
        );
        let client = tdw_core::http_support::build_client(USER_AGENT, "sec http client build")?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("sec cik_map extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec cik_map returned {status}: {body}"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec cik_map read body: {e}")))?;
        sleep(RATE_LIMIT_DELAY).await;
        Ok(bytes)
    }

    fn transform_data(&self, _query: &SecCikMapQuery, raw: Bytes) -> Result<Vec<SymbolMapping>> {
        // company_tickers.json is a JSON object keyed by string indices.
        let map: std::collections::BTreeMap<String, SecCompanyTicker> =
            serde_json::from_slice(&raw)
                .map_err(|e| Error::Provider(format!("sec cik_map parse_json: {e}")))?;
        let mut rows = Vec::with_capacity(map.len());
        for entry in map.into_values() {
            let (Some(cik), Some(symbol)) = (entry.cik_str, entry.ticker) else {
                continue;
            };
            if symbol.trim().is_empty() {
                continue;
            }
            rows.push(SymbolMapping {
                symbol: symbol.trim().to_ascii_uppercase(),
                cik: cik.to_string(),
                name: entry.title,
            });
        }
        Ok(rows)
    }
}

// ── Form 13F institutional-holdings fetcher (equity/ownership/form_13f) ────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC Form 13F-HR institutional-holdings filing-index fetcher.
    ///
    /// Calls `GET /submissions/CIK{cik_padded_10digits}.json` and selects the
    /// `13F-HR` filings, emitting one [`tdw_domain::OwnershipRecord`] per filing
    /// (`kind = "form_13f"`). A full position breakdown requires fetching each
    /// filing's information-table XML; this index path standardizes the filing
    /// list, which is the OpenBB-equivalent `equity/ownership/form_13f` shape.
    pub SecForm13FHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<SecFilingsQuery, OwnershipRecord> for SecForm13FHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "form_13f";

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
            .map_err(|e| Error::Provider(format!("sec form_13f extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec form_13f returned {status}: {body}"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec form_13f read body: {e}")))?;
        sleep(RATE_LIMIT_DELAY).await;
        Ok(bytes)
    }

    fn transform_data(&self, query: &SecFilingsQuery, raw: Bytes) -> Result<Vec<OwnershipRecord>> {
        let envelope: SecSubmissionsEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("sec form_13f parse_json: {e}")))?;
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
        let mut rows = Vec::new();
        for i in 0..len {
            let form = recent.form.get(i).cloned().unwrap_or_default();
            // Match 13F-HR and its amendments (13F-HR/A).
            if !form.to_ascii_uppercase().starts_with("13F-HR") {
                continue;
            }
            rows.push(OwnershipRecord {
                symbol: query.cik.clone(),
                kind: "form_13f".to_string(),
                holder: if entity_name.is_empty() {
                    None
                } else {
                    Some(entity_name.clone())
                },
                relationship: Some(form),
                date: recent.filing_date.get(i).cloned(),
                transaction_type: recent.accession_number.get(i).cloned(),
                shares: None,
                value: None,
                percentage: None,
            });
        }
        Ok(rows)
    }
}

// ── Fails-to-deliver fetcher (equity/shorts/fails_to_deliver) ─────────────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC fails-to-deliver fetcher.
    ///
    /// SEC publishes fails-to-deliver as semimonthly pipe-delimited data files
    /// (`https://www.sec.gov/data/foiadocs/failsdata.txt` style endpoints). The
    /// file set is large and the per-file URL is date-versioned; this fetcher
    /// parses a recorded pipe-delimited FTD extract into
    /// [`tdw_domain::OwnershipRecord`] rows (`kind = "fails_to_deliver"`) filtered
    /// to the queried symbol. Live retrieval of the date-versioned ZIP archive is
    /// out of scope for the keyless wave (documented limitation); the offline
    /// parse path is the standardized contract.
    pub SecFailsToDeliverHttpFetcher,
    WWW_BASE_URL
);

impl SecFailsToDeliverHttpFetcher {
    /// Parse SEC's pipe-delimited FTD format into ownership rows for `symbol`.
    ///
    /// The SEC FTD file header is
    /// `SETTLEMENT DATE|CUSIP|SYMBOL|QUANTITY (FAILS)|DESCRIPTION|PRICE`.
    /// Rows whose symbol does not match (case-insensitive) are skipped.
    fn parse_pipe_delimited(symbol: &str, text: &str) -> Vec<OwnershipRecord> {
        let want = symbol.trim().to_ascii_uppercase();
        let mut rows = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('|').collect();
            if fields.len() < 4 {
                continue;
            }
            // Skip the header row (non-numeric settlement date).
            let settlement = fields[0].trim();
            if settlement.eq_ignore_ascii_case("SETTLEMENT DATE") {
                continue;
            }
            let row_symbol = fields[2].trim().to_ascii_uppercase();
            if row_symbol != want {
                continue;
            }
            let shares = fields[3].trim().parse::<f64>().ok();
            let price = fields.get(5).and_then(|p| p.trim().parse::<f64>().ok());
            // SEC FTD settlement dates are YYYYMMDD; normalize to YYYY-MM-DD.
            let date = if settlement.len() == 8 && settlement.chars().all(|c| c.is_ascii_digit()) {
                Some(format!(
                    "{}-{}-{}",
                    &settlement[0..4],
                    &settlement[4..6],
                    &settlement[6..8]
                ))
            } else {
                Some(settlement.to_string())
            };
            rows.push(OwnershipRecord {
                symbol: want.clone(),
                kind: "fails_to_deliver".to_string(),
                holder: fields.get(4).map(|d| (*d).trim().to_string()),
                relationship: Some(fields[1].trim().to_string()),
                date,
                transaction_type: None,
                shares,
                value: price,
                percentage: None,
            });
        }
        rows
    }
}

#[async_trait]
impl Fetcher<SecHistoricalQuery, OwnershipRecord> for SecFailsToDeliverHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "fails_to_deliver";

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
        _query: &SecHistoricalQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        // The live FTD archive is a date-versioned ZIP of pipe-delimited files.
        // Resolving the exact archive URL is out of scope for the keyless wave;
        // surface a descriptive provider error so the offline parse path
        // (`transform_data` over a recorded extract) remains the standardized
        // contract and a live attempt fails clearly rather than silently.
        Err(Error::Provider(
            "sec fails_to_deliver live archive retrieval is not implemented; \
             supply a recorded pipe-delimited FTD extract via transform_data"
                .to_string(),
        ))
    }

    fn transform_data(
        &self,
        query: &SecHistoricalQuery,
        raw: Bytes,
    ) -> Result<Vec<OwnershipRecord>> {
        let text = String::from_utf8_lossy(&raw);
        Ok(Self::parse_pipe_delimited(&query.symbol, &text))
    }
}

// ── ETF holdings fetcher via N-PORT (etf/holdings) ────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC ETF-holdings fetcher backed by N-PORT (`NPORT-P`).
    ///
    /// Calls `GET /submissions/CIK{cik_padded_10digits}.json` to locate the most
    /// recent `NPORT-P` filing, then parses that filing's primary document XML
    /// for the `invstOrSec` holding entries, emitting one
    /// [`tdw_domain::EtfHolding`] per constituent. The XML is parsed with a small
    /// dependency-free tag extractor (matching the crate's no-extra-deps stance),
    /// scoped to the handful of fields N-PORT reports per holding.
    pub SecEtfHoldingsHttpFetcher,
    BASE_URL
);

/// One parsed `invstOrSec` N-PORT holding entry.
struct NportHolding {
    name: String,
    cusip: Option<String>,
    isin: Option<String>,
    balance: Option<f64>,
    value_usd: Option<f64>,
    weight_pct: Option<f64>,
}

impl SecEtfHoldingsHttpFetcher {
    /// Extract the text of the first `<tag>…</tag>` inside `xml`, trimmed.
    /// Returns `None` when the tag is absent or empty. Dependency-free: a linear
    /// scan over the slice, sufficient for the flat N-PORT holding fields.
    fn tag_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let start = xml.find(&open)? + open.len();
        let end = xml[start..].find(&close)? + start;
        let text = xml[start..end].trim();
        if text.is_empty() { None } else { Some(text) }
    }

    /// Extract the value of a self-closing `<tag .. value="X" ..>` attribute (the
    /// N-PORT `identifiers` block uses `<isin value="…"/>` / `<cusip value="…"/>`).
    fn attr_value(fragment: &str, tag: &str) -> Option<String> {
        let open = format!("<{tag}");
        let tag_start = fragment.find(&open)?;
        let rest = &fragment[tag_start..];
        let tag_end = rest.find('>')?;
        let attrs = &rest[..tag_end];
        let key = "value=\"";
        let v_start = attrs.find(key)? + key.len();
        let v_end = attrs[v_start..].find('"')? + v_start;
        let value = attrs[v_start..v_end].trim();
        if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        }
    }

    /// Parse the `invstOrSec` holding blocks out of an N-PORT primary-document
    /// XML body. Each block is bounded by `<invstOrSec>`…`</invstOrSec>`.
    fn parse_nport_holdings(xml: &str) -> Vec<NportHolding> {
        let open = "<invstOrSec>";
        let close = "</invstOrSec>";
        let mut holdings = Vec::new();
        let mut cursor = 0usize;
        while let Some(rel_start) = xml[cursor..].find(open) {
            let start = cursor + rel_start + open.len();
            let Some(rel_end) = xml[start..].find(close) else {
                break;
            };
            let block = &xml[start..start + rel_end];
            cursor = start + rel_end + close.len();

            let name = Self::tag_text(block, "name").unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }
            // CUSIP is usually a plain element; ISIN lives in <identifiers>.
            let cusip = Self::tag_text(block, "cusip")
                .map(str::to_string)
                .or_else(|| Self::attr_value(block, "cusip"));
            let isin = Self::attr_value(block, "isin");
            let balance = Self::tag_text(block, "balance").and_then(|v| v.parse().ok());
            let value_usd = Self::tag_text(block, "valUSD").and_then(|v| v.parse().ok());
            let weight_pct = Self::tag_text(block, "pctVal").and_then(|v| v.parse().ok());
            holdings.push(NportHolding {
                name,
                cusip,
                isin,
                balance,
                value_usd,
                weight_pct,
            });
        }
        holdings
    }
}

#[async_trait]
impl Fetcher<SecFilingsQuery, EtfHolding> for SecEtfHoldingsHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "etf_holdings";

    fn transform_query(params: Value) -> Result<SecFilingsQuery> {
        let cik = params
            .get("cik")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("sec cik must be a string".to_string()))?;
        SecFilingsQuery::new(cik).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    async fn extract_data(&self, query: &SecFilingsQuery, _creds: &Credentials) -> Result<Bytes> {
        // Step 1: locate the most recent NPORT-P filing from the submissions
        // index. Step 2: fetch that filing's primary-document XML. The two-step
        // EDGAR navigation matches the documented submissions → filing-document
        // flow.
        let submissions_url = format!(
            "{}/submissions/CIK{}.json",
            self.base_url().trim_end_matches('/'),
            query.padded_cik(),
        );
        let client = tdw_core::http_support::build_client(USER_AGENT, "sec http client build")?;
        let response = client
            .get(&submissions_url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("sec etf_holdings submissions: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec etf_holdings submissions returned {status}: {body}"
            )));
        }
        let submissions = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec etf_holdings read submissions: {e}")))?;
        sleep(RATE_LIMIT_DELAY).await;

        let envelope: SecSubmissionsEnvelope = serde_json::from_slice(&submissions)
            .map_err(|e| Error::Provider(format!("sec etf_holdings parse submissions: {e}")))?;
        let recent = envelope
            .filings
            .and_then(|f| f.recent)
            .ok_or_else(|| Error::Provider("sec etf_holdings: no recent filings".to_string()))?;
        let nport_index = recent
            .form
            .iter()
            .position(|f| f.eq_ignore_ascii_case("NPORT-P"))
            .ok_or_else(|| {
                Error::Provider("sec etf_holdings: no NPORT-P filing found".to_string())
            })?;
        let accession = recent.accession_number.get(nport_index).ok_or_else(|| {
            Error::Provider("sec etf_holdings: NPORT-P accession missing".to_string())
        })?;
        // The primary document for an N-PORT filing is `primary_doc.xml`. The
        // EDGAR archives path drops the dashes from the accession number.
        let accession_nodash = accession.replace('-', "");
        let cik_unpadded = query.cik.trim_start_matches('0');
        let cik_unpadded = if cik_unpadded.is_empty() {
            "0"
        } else {
            cik_unpadded
        };
        let doc_url = format!(
            "{WWW_BASE_URL}/Archives/edgar/data/{cik_unpadded}/{accession_nodash}/primary_doc.xml",
        );
        let doc_response = client
            .get(&doc_url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("sec etf_holdings primary_doc: {e}")))?;
        if !doc_response.status().is_success() {
            let status = doc_response.status();
            let body = doc_response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec etf_holdings primary_doc returned {status}: {body}"
            )));
        }
        let xml = doc_response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec etf_holdings read primary_doc: {e}")))?;
        sleep(RATE_LIMIT_DELAY).await;
        Ok(xml)
    }

    fn transform_data(&self, query: &SecFilingsQuery, raw: Bytes) -> Result<Vec<EtfHolding>> {
        let xml = String::from_utf8_lossy(&raw);
        let report_date = Self::tag_text(&xml, "repPdDate").map(str::to_string);
        let holdings = Self::parse_nport_holdings(&xml);
        Ok(holdings
            .into_iter()
            .map(|h| EtfHolding {
                fund_symbol: query.cik.clone(),
                cik: Some(query.cik.clone()),
                report_date: report_date.clone(),
                holding_name: h.name,
                cusip: h.cusip,
                isin: h.isin,
                balance: h.balance,
                value_usd: h.value_usd,
                weight_pct: h.weight_pct,
            })
            .collect())
    }
}

// ── CIK → ticker symbol map fetcher (regulators/sec/symbol_map) ───────────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC `company_tickers.json` CIK→ticker map fetcher.
    ///
    /// Shares the same keyless `company_tickers.json` source as the cik_map
    /// fetcher but is registered under its own `symbol_map` endpoint key so the
    /// CIK→ticker direction has a distinct catalog route. Standardizes
    /// `regulators/sec/symbol_map` to [`tdw_domain::SymbolMapping`] rows.
    pub SecSymbolMapHttpFetcher,
    WWW_BASE_URL
);

#[async_trait]
impl Fetcher<SecCikMapQuery, SymbolMapping> for SecSymbolMapHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "symbol_map";

    fn transform_query(_params: Value) -> Result<SecCikMapQuery> {
        Ok(SecCikMapQuery::new())
    }

    async fn extract_data(&self, _query: &SecCikMapQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_company_tickers(self.base_url()).await
    }

    fn transform_data(&self, _query: &SecCikMapQuery, raw: Bytes) -> Result<Vec<SymbolMapping>> {
        parse_company_tickers(&raw)
    }
}

// ── Institution-name search fetcher (regulators/sec/institutions_search) ──────

tdw_core::provider_fetcher_struct!(
    /// Production SEC institutions-search fetcher.
    ///
    /// Filters the keyless `company_tickers.json` directory by a name needle,
    /// emitting one [`tdw_domain::SecInstitution`] per match. Standardizes
    /// `regulators/sec/institutions_search`.
    pub SecInstitutionsSearchHttpFetcher,
    WWW_BASE_URL
);

#[async_trait]
impl Fetcher<SecSearchQuery, SecInstitution> for SecInstitutionsSearchHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "institutions_search";

    fn transform_query(params: Value) -> Result<SecSearchQuery> {
        let query = params
            .get("query")
            .or_else(|| params.get("q"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(SecSearchQuery::new(query))
    }

    async fn extract_data(&self, _query: &SecSearchQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_company_tickers(self.base_url()).await
    }

    fn transform_data(&self, query: &SecSearchQuery, raw: Bytes) -> Result<Vec<SecInstitution>> {
        let map: std::collections::BTreeMap<String, SecCompanyTicker> =
            serde_json::from_slice(&raw)
                .map_err(|e| Error::Provider(format!("sec institutions_search parse_json: {e}")))?;
        let needle = query.query.to_ascii_lowercase();
        let mut rows = Vec::new();
        for entry in map.into_values() {
            let Some(cik) = entry.cik_str else { continue };
            let Some(name) = entry.title else { continue };
            if name.trim().is_empty() {
                continue;
            }
            if !needle.is_empty() && !name.to_ascii_lowercase().contains(&needle) {
                continue;
            }
            rows.push(SecInstitution {
                cik: cik.to_string(),
                name: name.trim().to_string(),
                symbol: entry
                    .ticker
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| t.trim().to_ascii_uppercase()),
            });
        }
        Ok(rows)
    }
}

// ── SIC industry-code search fetcher (regulators/sec/sic_search) ──────────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC SIC-code search fetcher.
    ///
    /// Filters the SEC-published Standard Industrial Classification (SIC) code
    /// list (an embedded static table — the list is a small, stable public
    /// reference with no JSON API) by a query needle matched against the code or
    /// description. Standardizes `regulators/sec/sic_search`.
    pub SecSicSearchHttpFetcher,
    WWW_BASE_URL
);

#[async_trait]
impl Fetcher<SecSearchQuery, SicCode> for SecSicSearchHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "sic_search";

    fn transform_query(params: Value) -> Result<SecSearchQuery> {
        let query = params
            .get("query")
            .or_else(|| params.get("q"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(SecSearchQuery::new(query))
    }

    async fn extract_data(&self, _query: &SecSearchQuery, _creds: &Credentials) -> Result<Bytes> {
        // The SIC list is an embedded static reference (no SEC JSON API), so no
        // network call is made: `transform_data` filters the embedded table
        // directly. Returning empty bytes keeps the fetcher offline-deterministic
        // and keyless while satisfying the `Fetcher` extract→transform contract.
        Ok(Bytes::new())
    }

    fn transform_data(&self, query: &SecSearchQuery, _raw: Bytes) -> Result<Vec<SicCode>> {
        let needle = query.query.to_ascii_lowercase();
        Ok(crate::sic::SIC_CODES
            .iter()
            .filter(|e| {
                needle.is_empty()
                    || e.code.contains(&needle)
                    || e.description.to_ascii_lowercase().contains(&needle)
            })
            .map(|e| SicCode {
                code: e.code.to_string(),
                description: e.description.to_string(),
                office: e.office.map(str::to_string),
            })
            .collect())
    }
}

// ── Filing index helpers + header / schema-file fetchers ──────────────────────

/// Wire shape of the EDGAR `{accession}-index.json` document.
#[derive(Deserialize)]
struct SecFilingIndexEnvelope {
    #[serde(default)]
    directory: Option<SecFilingDirectory>,
}

#[derive(Deserialize)]
struct SecFilingDirectory {
    #[serde(default)]
    item: Vec<SecFilingItem>,
}

#[derive(Deserialize)]
struct SecFilingItem {
    #[serde(default)]
    name: Option<String>,
    #[serde(rename = "type", default)]
    item_type: Option<String>,
    #[serde(default)]
    size: Option<serde_json::Value>,
    #[serde(rename = "last-modified", default)]
    last_modified: Option<String>,
}

/// Build the EDGAR filing-index URL for a (cik, accession) pair. The archive
/// path uses the unpadded CIK and the dash-stripped accession number.
fn filing_index_url(base_url: &str, query: &SecAccessionQuery) -> String {
    let cik_unpadded = query.cik.trim_start_matches('0');
    let cik_unpadded = if cik_unpadded.is_empty() {
        "0"
    } else {
        cik_unpadded
    };
    format!(
        "{}/Archives/edgar/data/{cik_unpadded}/{}/index.json",
        base_url.trim_end_matches('/'),
        query.accession_nodash(),
    )
}

/// Shared GET for an EDGAR filing-index JSON document.
async fn fetch_filing_index(base_url: &str, query: &SecAccessionQuery) -> Result<Bytes> {
    let url = filing_index_url(base_url, query);
    let client = tdw_core::http_support::build_client(USER_AGENT, "sec http client build")?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::Provider(format!("sec filing index extract_data: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "sec filing index returned {status}: {body}"
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("sec filing index read body: {e}")))?;
    sleep(RATE_LIMIT_DELAY).await;
    Ok(bytes)
}

tdw_core::provider_fetcher_struct!(
    /// Production SEC filing-header fetcher.
    ///
    /// Reads the EDGAR `{accession}/index.json` header block and emits one
    /// [`tdw_domain::FilingHeader`] row. Standardizes
    /// `regulators/sec/filing_headers`. The www.sec.gov archive hosts the index.
    pub SecFilingHeadersHttpFetcher,
    WWW_BASE_URL
);

#[async_trait]
impl Fetcher<SecAccessionQuery, FilingHeader> for SecFilingHeadersHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "filing_headers";

    fn transform_query(params: Value) -> Result<SecAccessionQuery> {
        accession_query_from_params(&params)
    }

    async fn extract_data(&self, query: &SecAccessionQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_filing_index(self.base_url(), query).await
    }

    fn transform_data(&self, query: &SecAccessionQuery, raw: Bytes) -> Result<Vec<FilingHeader>> {
        // EDGAR index.json carries metadata in the top-level object alongside
        // `directory`. We extract the header anchors that the index reliably
        // reports; richer header fields live in the filing's SGML header, which
        // is out of scope for this keyless index-backed route.
        let value: Value = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("sec filing_headers parse_json: {e}")))?;
        let form_type = value
            .get("directory")
            .and_then(|d| d.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string);
        Ok(vec![FilingHeader {
            cik: query.cik.clone(),
            accession_number: query.accession.clone(),
            form_type: value
                .get("formType")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(form_type),
            filing_date: value
                .get("filingDate")
                .and_then(Value::as_str)
                .map(str::to_string),
            period_of_report: value
                .get("periodOfReport")
                .and_then(Value::as_str)
                .map(str::to_string),
            description: value
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string),
        }])
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production SEC schema/data-file list fetcher.
    ///
    /// Reads the EDGAR `{accession}/index.json` `directory.item[]` array and
    /// emits one [`tdw_domain::FilingFile`] per document/schema file in the
    /// filing. Standardizes `regulators/sec/schema_files`.
    pub SecSchemaFilesHttpFetcher,
    WWW_BASE_URL
);

#[async_trait]
impl Fetcher<SecAccessionQuery, FilingFile> for SecSchemaFilesHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "schema_files";

    fn transform_query(params: Value) -> Result<SecAccessionQuery> {
        accession_query_from_params(&params)
    }

    async fn extract_data(&self, query: &SecAccessionQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_filing_index(self.base_url(), query).await
    }

    fn transform_data(&self, _query: &SecAccessionQuery, raw: Bytes) -> Result<Vec<FilingFile>> {
        let envelope: SecFilingIndexEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("sec schema_files parse_json: {e}")))?;
        let items = envelope.directory.map(|d| d.item).unwrap_or_default();
        Ok(items
            .into_iter()
            .filter_map(|item| {
                let name = item.name?;
                if name.trim().is_empty() {
                    return None;
                }
                // EDGAR reports `size` as either a number or a numeric string.
                let size = item.size.and_then(|s| match s {
                    Value::Number(n) => n.as_u64(),
                    Value::String(s) => s.trim().parse::<u64>().ok(),
                    _ => None,
                });
                Some(FilingFile {
                    name: name.trim().to_string(),
                    file_type: item.item_type.filter(|t| !t.trim().is_empty()),
                    size,
                    last_modified: item.last_modified.filter(|t| !t.trim().is_empty()),
                })
            })
            .collect())
    }
}

/// Parse a `(cik, accession)` accession query from request params, accepting
/// either an explicit `accession` field or an `accession_number` alias.
fn accession_query_from_params(params: &Value) -> Result<SecAccessionQuery> {
    let cik = params
        .get("cik")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidQuery("sec cik must be a string".to_string()))?;
    let accession = params
        .get("accession")
        .or_else(|| params.get("accession_number"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidQuery("sec accession must be a string".to_string()))?;
    SecAccessionQuery::new(cik, accession).map_err(|e| Error::InvalidQuery(e.to_string()))
}

// ── Litigation-releases RSS fetcher (regulators/sec/rss_litigation) ───────────

tdw_core::provider_fetcher_struct!(
    /// Production SEC litigation-releases RSS fetcher.
    ///
    /// Reads the public SEC litigation RSS feed and emits one
    /// [`tdw_domain::LitigationRelease`] per `<item>`. Standardizes
    /// `regulators/sec/rss_litigation`. A dependency-free tag scan parses the
    /// flat RSS item fields (matching the crate's no-extra-deps stance).
    pub SecRssLitigationHttpFetcher,
    WWW_BASE_URL
);

#[async_trait]
impl Fetcher<SecCikMapQuery, LitigationRelease> for SecRssLitigationHttpFetcher {
    const PROVIDER: &'static str = "sec";
    const ENDPOINT: &'static str = "rss_litigation";

    fn transform_query(_params: Value) -> Result<SecCikMapQuery> {
        Ok(SecCikMapQuery::new())
    }

    async fn extract_data(&self, _query: &SecCikMapQuery, _creds: &Credentials) -> Result<Bytes> {
        let url = format!(
            "{}/rss/litigation/litigation.xml",
            self.base_url().trim_end_matches('/'),
        );
        let client = tdw_core::http_support::build_client(USER_AGENT, "sec http client build")?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("sec rss_litigation extract_data: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "sec rss_litigation returned {status}: {body}"
            )));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("sec rss_litigation read body: {e}")))?;
        sleep(RATE_LIMIT_DELAY).await;
        Ok(bytes)
    }

    fn transform_data(
        &self,
        _query: &SecCikMapQuery,
        raw: Bytes,
    ) -> Result<Vec<LitigationRelease>> {
        let xml = String::from_utf8_lossy(&raw);
        Ok(parse_rss_items(&xml))
    }
}

/// Extract the trimmed text of the first `<tag>…</tag>` in `xml`, unwrapping a
/// surrounding `<![CDATA[…]]>` block when present. Returns `None` when absent or
/// empty. Dependency-free linear scan, sufficient for the flat RSS item fields.
fn rss_tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let mut text = xml[start..end].trim();
    if let Some(stripped) = text.strip_prefix("<![CDATA[") {
        text = stripped.strip_suffix("]]>").unwrap_or(stripped);
    }
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Parse the `<item>` blocks of an RSS feed into litigation-release rows.
fn parse_rss_items(xml: &str) -> Vec<LitigationRelease> {
    let open = "<item>";
    let close = "</item>";
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_start) = xml[cursor..].find(open) {
        let start = cursor + rel_start + open.len();
        let Some(rel_end) = xml[start..].find(close) else {
            break;
        };
        let block = &xml[start..start + rel_end];
        cursor = start + rel_end + close.len();

        let (Some(title), Some(link)) = (rss_tag_text(block, "title"), rss_tag_text(block, "link"))
        else {
            continue;
        };
        rows.push(LitigationRelease {
            title,
            link,
            published: rss_tag_text(block, "pubDate"),
            summary: rss_tag_text(block, "description"),
        });
    }
    rows
}

// ── Shared company_tickers.json helpers ───────────────────────────────────────

/// Shared GET for the keyless `company_tickers.json` directory.
async fn fetch_company_tickers(base_url: &str) -> Result<Bytes> {
    let url = format!(
        "{}/files/company_tickers.json",
        base_url.trim_end_matches('/'),
    );
    let client = tdw_core::http_support::build_client(USER_AGENT, "sec http client build")?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| Error::Provider(format!("sec company_tickers extract_data: {e}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "sec company_tickers returned {status}: {body}"
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| Error::Provider(format!("sec company_tickers read body: {e}")))?;
    sleep(RATE_LIMIT_DELAY).await;
    Ok(bytes)
}

/// Parse `company_tickers.json` (an object keyed by integer-as-string indices)
/// into [`SymbolMapping`] rows.
fn parse_company_tickers(raw: &Bytes) -> Result<Vec<SymbolMapping>> {
    let map: std::collections::BTreeMap<String, SecCompanyTicker> = serde_json::from_slice(raw)
        .map_err(|e| Error::Provider(format!("sec company_tickers parse_json: {e}")))?;
    let mut rows = Vec::with_capacity(map.len());
    for entry in map.into_values() {
        let (Some(cik), Some(symbol)) = (entry.cik_str, entry.ticker) else {
            continue;
        };
        if symbol.trim().is_empty() {
            continue;
        }
        rows.push(SymbolMapping {
            symbol: symbol.trim().to_ascii_uppercase(),
            cik: cik.to_string(),
            name: entry.title,
        });
    }
    Ok(rows)
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
                "cik": 320_193,
                "entityName": "Apple Inc.",
                "facts": {
                    "us-gaap": {
                        "Revenue": {
                            "label": "Revenue",
                            "units": {
                                "USD": [
                                    {"end": "2024-09-28", "val": 391_035_000_000.0_f64, "form": "10-K"},
                                    {"end": "2023-09-30", "val": 383_285_000_000.0_f64, "form": "10-K"},
                                    {"end": "2024-03-30", "val": 90_753_000_000.0_f64, "form": "10-Q"}
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
        assert!((rows[0].close - 391_035_000_000.0_f64).abs() < 1.0);
        assert_eq!(rows[0].source, "sec-xbrl");
        assert_eq!(rows[1].ts, "2023-09-30T00:00:00Z");
    }

    #[test]
    fn transform_data_xbrl_returns_empty_when_no_revenue_concept() {
        let fetcher = SecXbrlHttpFetcher::default();
        let query = historical_query();
        let raw = Bytes::from(
            serde_json::json!({
                "cik": 320_193,
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

    #[test]
    fn transform_data_company_facts_flattens_taxonomy_tree() {
        let fetcher = SecCompanyFactsHttpFetcher::default();
        let query = filings_query();
        let raw = Bytes::from(
            serde_json::json!({
                "cik": 320_193,
                "entityName": "Apple Inc.",
                "facts": {
                    "us-gaap": {
                        "Revenues": {
                            "label": "Revenues",
                            "units": {
                                "USD": [
                                    {"end": "2024-09-28", "val": 391_035_000_000.0_f64,
                                     "fy": 2024, "fp": "FY", "form": "10-K"},
                                    {"end": "2023-09-30", "val": 383_285_000_000.0_f64,
                                     "fy": 2023, "fp": "FY", "form": "10-K"}
                                ]
                            }
                        }
                    },
                    "dei": {
                        "EntityCommonStockSharesOutstanding": {
                            "units": {
                                "shares": [
                                    {"end": "2024-10-18", "val": 15_115_823_000.0_f64,
                                     "fy": 2024, "fp": "FY", "form": "10-K"}
                                ]
                            }
                        },
                        // A concept with no `units` key must be skipped cleanly by
                        // the single-pass typed deserializer (no `Value` round-trip).
                        "EntityRegistrantName": {
                            "label": "Entity Registrant Name"
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
        assert_eq!(rows.len(), 3, "rows={rows:#?}");
        assert!(
            !rows.iter().any(|r| r.concept == "EntityRegistrantName"),
            "concept without units must be skipped",
        );
        let revenue = rows
            .iter()
            .find(|r| r.concept == "Revenues" && r.end.as_deref() == Some("2024-09-28"))
            .expect("revenue fact present");
        assert_eq!(revenue.cik, "320193");
        assert_eq!(revenue.taxonomy, "us-gaap");
        assert_eq!(revenue.unit.as_deref(), Some("USD"));
        assert_eq!(revenue.fiscal_year, Some(2024));
        assert_eq!(revenue.form.as_deref(), Some("10-K"));
        assert!((revenue.value.unwrap_or_default() - 391_035_000_000.0).abs() < 1.0);
        assert!(rows.iter().any(|r| r.taxonomy == "dei"));
    }

    #[test]
    fn transform_data_company_facts_empty_when_no_facts() {
        let fetcher = SecCompanyFactsHttpFetcher::default();
        let query = filings_query();
        let raw = Bytes::from(
            serde_json::json!({"cik": 320_193, "entityName": "Apple Inc."})
                .to_string()
                .into_bytes(),
        );
        let rows = fetcher
            .transform_data(&query, raw)
            .expect("empty facts must not error");
        assert!(rows.is_empty());
    }

    #[test]
    fn transform_data_latest_financial_reports_filters_to_periodic_forms() {
        let fetcher = SecLatestFinancialReportsHttpFetcher::default();
        let query = filings_query();
        let raw = Bytes::from(
            serde_json::json!({
                "cik": "320193",
                "name": "Apple Inc.",
                "filings": {
                    "recent": {
                        "accessionNumber": ["0000320193-24-000123", "0000320193-24-000099",
                                            "0000320193-24-000050"],
                        "form": ["10-K", "8-K", "10-Q"],
                        "filingDate": ["2024-11-01", "2024-10-15", "2024-08-01"]
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let rows = fetcher
            .transform_data(&query, raw)
            .expect("transform_data must succeed");
        assert_eq!(rows.len(), 2, "only 10-K and 10-Q kept: {rows:#?}");
        assert!(
            rows.iter()
                .all(|r| r.form_type == "10-K" || r.form_type == "10-Q")
        );
        assert_eq!(rows[0].symbol, "320193");
        assert_eq!(rows[0].cik.as_deref(), Some("320193"));
    }

    #[test]
    fn transform_query_company_facts_requires_cik() {
        assert!(SecCompanyFactsHttpFetcher::transform_query(serde_json::json!({})).is_err());
        let q = SecCompanyFactsHttpFetcher::transform_query(serde_json::json!({"cik": "320193"}))
            .expect("valid cik");
        assert_eq!(q.padded_cik(), "0000320193");
    }

    #[test]
    fn transform_data_nport_disclosure_filters_to_nport_forms() {
        let fetcher = SecNportDisclosureHttpFetcher::default();
        let query = filings_query();
        let raw = Bytes::from(
            serde_json::json!({
                "cik": "884394",
                "name": "SPDR S&P 500 ETF Trust",
                "filings": {
                    "recent": {
                        "accessionNumber": ["0000884394-24-000010", "0000884394-24-000008",
                                            "0000884394-24-000005"],
                        "form": ["NPORT-P", "8-K", "NPORT-EX"],
                        "filingDate": ["2024-11-01", "2024-10-15", "2024-08-01"]
                    }
                }
            })
            .to_string()
            .into_bytes(),
        );
        let rows = fetcher
            .transform_data(&query, raw)
            .expect("transform_data must succeed");
        assert_eq!(rows.len(), 2, "only NPORT-P and NPORT-EX kept: {rows:#?}");
        assert!(
            rows.iter()
                .all(|r| r.form_type == "NPORT-P" || r.form_type == "NPORT-EX")
        );
        assert_eq!(rows[0].cik.as_deref(), Some("884394"));
    }

    #[test]
    fn transform_query_nport_disclosure_requires_cik() {
        assert!(SecNportDisclosureHttpFetcher::transform_query(serde_json::json!({})).is_err());
        let q =
            SecNportDisclosureHttpFetcher::transform_query(serde_json::json!({"cik": "884394"}))
                .expect("valid cik");
        assert_eq!(q.padded_cik(), "0000884394");
    }
}
