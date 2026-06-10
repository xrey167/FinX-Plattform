//! Real US Treasury `FiscalData` HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public `FiscalData` API
//! (`https://api.fiscaldata.treasury.gov`) without authentication. Live calls
//! are gated by `TDW_GOVERNMENT_US_LIVE=1` in the integration tests.
//!
//! The `FiscalData` API returns a `{ "data": [ … ], "meta": { … } }` envelope; the
//! fetchers decode the `data` array into standardized [`tdw_domain`] rows.

#![cfg(feature = "http")]

use serde::Deserialize;
use tdw_core::http_support::prelude::*;
use tdw_core::query_params::StandardParams;
use tdw_domain::{TreasuryAuction, TreasuryPrice};

use crate::{BASE_URL, GovUsCatalogQuery};

const USER_AGENT: &str = "tdw-provider-government-us/0.1 (contact@finx.example)";

/// Decode a single `FiscalData` field as an `f64`, accepting both JSON numbers and
/// the numeric strings the `FiscalData` API frequently returns. Treats the
/// `FiscalData` "null" sentinels (`""`, `"null"`, `"*"`) as absent.
fn parse_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") || trimmed == "*" {
                None
            } else {
                trimmed.replace(',', "").parse::<f64>().ok()
            }
        }
        _ => None,
    }
}

/// Read a `FiscalData` string field, treating empty / "null" as absent.
fn parse_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("null"))
        .map(str::to_string)
}

/// Build the `FiscalData` request query parameters from the shared params:
/// page size (`page[size]`) and an optional `record_date` range filter.
fn fiscal_query_params(params: &StandardParams) -> Vec<(String, String)> {
    let mut query = Vec::new();
    let limit = params.limit.unwrap_or(0);
    if limit > 0 {
        query.push(("page[size]".to_string(), limit.to_string()));
    }
    let mut filters = Vec::new();
    if let Some(start) = params.start_date {
        filters.push(format!("record_date:gte:{start}"));
    }
    if let Some(end) = params.end_date {
        filters.push(format!("record_date:lte:{end}"));
    }
    if !filters.is_empty() {
        query.push(("filter".to_string(), filters.join(",")));
    }
    query
}

/// Perform a `FiscalData` GET for `dataset`, returning the raw response bytes.
async fn fetch_dataset(
    base_url: &str,
    dataset: &str,
    params: &StandardParams,
    ctx: &str,
) -> Result<Bytes> {
    let endpoint = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        dataset.trim_start_matches('/')
    );
    let client =
        tdw_core::http_support::build_client(USER_AGENT, "government_us http client build")?;
    let response = client
        .get(&endpoint)
        .query(&fiscal_query_params(params))
        .send()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} extract_data: {error}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!("{ctx} returned {status}: {body}")));
    }
    response
        .bytes()
        .await
        .map_err(|error| Error::Provider(format!("{ctx} read body: {error}")))
}

/// The `FiscalData` `{ "data": [ … ] }` envelope. Each record is decoded as an
/// untyped JSON object so the heterogeneous, partly-string-valued `FiscalData`
/// columns can be coerced per field.
#[derive(Deserialize)]
struct FiscalEnvelope {
    #[serde(default)]
    data: Vec<serde_json::Map<String, Value>>,
    #[serde(default)]
    error: Option<String>,
}

fn decode_envelope(raw: &Bytes, ctx: &str) -> Result<Vec<serde_json::Map<String, Value>>> {
    let envelope: FiscalEnvelope = serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("{ctx} parse_json: {error}")))?;
    if let Some(error) = envelope.error {
        return Err(Error::Provider(format!("{ctx} api error: {error}")));
    }
    Ok(envelope.data)
}

// ── Treasury auctions fetcher ─────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production FiscalData Treasury-auctions fetcher.
    ///
    /// Standardizes `fixedincome/government/treasury_auctions` to
    /// [`tdw_domain::TreasuryAuction`] rows from the FiscalData
    /// `auctions_query` dataset.
    pub GovUsTreasuryAuctionsHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<GovUsCatalogQuery, TreasuryAuction> for GovUsTreasuryAuctionsHttpFetcher {
    const PROVIDER: &'static str = "government_us";
    const ENDPOINT: &'static str = "treasury_auctions";

    fn transform_query(params: Value) -> Result<GovUsCatalogQuery> {
        GovUsCatalogQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &GovUsCatalogQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_dataset(
            self.base_url(),
            &query.dataset,
            &query.params,
            "government_us auctions",
        )
        .await
    }

    fn transform_data(
        &self,
        _query: &GovUsCatalogQuery,
        raw: Bytes,
    ) -> Result<Vec<TreasuryAuction>> {
        let records = decode_envelope(&raw, "government_us auctions")?;
        let mut rows = Vec::with_capacity(records.len());
        for record in records {
            let Some(cusip) = parse_string(record.get("cusip")) else {
                continue;
            };
            let Some(auction_date) = parse_string(record.get("auction_date")) else {
                continue;
            };
            rows.push(TreasuryAuction {
                cusip,
                security_type: parse_string(record.get("security_type")),
                security_term: parse_string(record.get("security_term")),
                auction_date,
                issue_date: parse_string(record.get("issue_date")),
                maturity_date: parse_string(record.get("maturity_date")),
                high_yield: parse_f64(record.get("high_yield")),
                interest_rate: parse_f64(record.get("interest_rate")),
                offering_amount: parse_f64(record.get("offering_amount")),
                bid_to_cover_ratio: parse_f64(record.get("bid_to_cover_ratio")),
            });
        }
        Ok(rows)
    }
}

// ── Treasury prices fetcher ───────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production FiscalData Treasury-prices fetcher.
    ///
    /// Standardizes `fixedincome/government/treasury_prices` to
    /// [`tdw_domain::TreasuryPrice`] rows from the FiscalData securities dataset.
    pub GovUsTreasuryPricesHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<GovUsCatalogQuery, TreasuryPrice> for GovUsTreasuryPricesHttpFetcher {
    const PROVIDER: &'static str = "government_us";
    const ENDPOINT: &'static str = "treasury_prices";

    fn transform_query(params: Value) -> Result<GovUsCatalogQuery> {
        GovUsCatalogQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &GovUsCatalogQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_dataset(
            self.base_url(),
            &query.dataset,
            &query.params,
            "government_us prices",
        )
        .await
    }

    fn transform_data(&self, _query: &GovUsCatalogQuery, raw: Bytes) -> Result<Vec<TreasuryPrice>> {
        let records = decode_envelope(&raw, "government_us prices")?;
        let mut rows = Vec::with_capacity(records.len());
        for record in records {
            let Some(cusip) = parse_string(record.get("cusip")) else {
                continue;
            };
            // `FiscalData` uses `record_date` as the observation date column.
            let Some(date) = parse_string(record.get("record_date"))
                .or_else(|| parse_string(record.get("date")))
            else {
                continue;
            };
            rows.push(TreasuryPrice {
                cusip,
                security_type: parse_string(record.get("security_type")),
                date,
                price: parse_f64(record.get("price"))
                    .or_else(|| parse_f64(record.get("end_of_day"))),
                coupon_rate: parse_f64(record.get("interest_rate"))
                    .or_else(|| parse_f64(record.get("coupon_rate"))),
                maturity_date: parse_string(record.get("maturity_date")),
            });
        }
        Ok(rows)
    }
}
