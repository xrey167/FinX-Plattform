//! Real `congress.gov` v3 HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public `congress.gov` v3
//! API (`https://api.congress.gov/v3`). The API requires a free api.data.gov
//! key (read from `CONGRESS_GOV_API_KEY`) passed as the `api_key` query
//! parameter alongside `format=json`. Live calls are gated by
//! `TDW_CONGRESS_GOV_LIVE=1` in the integration tests.
//!
//! Each fetcher resolves its catalog `route_template`, substitutes the
//! `congress`/`billType`/`billNumber` path parameters from the query, performs
//! the GET, and normalizes the JSON response to a standardized [`tdw_domain`]
//! model so no raw `congress.gov` shape leaks out.

#![cfg(feature = "http")]

use tdw_core::http_support::prelude::*;
use tdw_domain::{BillTextUrl, CongressBill};

use crate::{API_KEY_ENV, BASE_URL, CongressBillQuery};

const USER_AGENT: &str = "tdw-provider-congress-gov/0.1 (contact@finx.example)";

/// Read the required api.data.gov key, erroring when `CONGRESS_GOV_API_KEY` is
/// unset, through the shared `read_required_key` helper.
fn read_api_key() -> Result<String> {
    tdw_core::http_support::read_required_key(API_KEY_ENV, "congress-gov")
}

/// Read an optional string field from a JSON object, treating empty as absent.
fn opt_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Substitute the `{congress}`/`{billType}`/`{billNumber}` placeholders in a
/// route template with the query's path parameters.
fn fill_route_template(query: &CongressBillQuery) -> String {
    let mut route = query
        .endpoint()
        .route_template
        .replace("{congress}", &query.congress.to_string())
        .replace("{billType}", &query.bill_type);
    if let Some(number) = &query.bill_number {
        route = route.replace("{billNumber}", number);
    }
    route
}

/// Perform a `congress.gov` GET for the query's resolved route, attaching the
/// required `api_key`, `format=json`, and optional `limit`, and return the raw
/// response bytes.
async fn fetch_congress(base_url: &str, query: &CongressBillQuery, ctx: &str) -> Result<Bytes> {
    let api_key = read_api_key()?;
    let route = fill_route_template(query);
    let endpoint = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        route.trim_start_matches('/')
    );
    let client =
        tdw_core::http_support::build_client(USER_AGENT, "congress-gov http client build")?;

    let mut params: Vec<(String, String)> = vec![
        ("format".to_string(), "json".to_string()),
        ("api_key".to_string(), api_key),
    ];
    let limit = query.params.limit.unwrap_or(0);
    if limit > 0 {
        params.push(("limit".to_string(), limit.to_string()));
    }

    let response = client
        .get(&endpoint)
        .query(&params)
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

/// Decode a `congress.gov` response body into a top-level JSON object.
fn decode_object(raw: &Bytes, ctx: &str) -> Result<serde_json::Map<String, Value>> {
    match serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("{ctx} parse_json: {error}")))?
    {
        Value::Object(map) => Ok(map),
        _ => Err(Error::Provider(format!(
            "{ctx} parse_json: expected a JSON object"
        ))),
    }
}

/// Normalize one `congress.gov` bill JSON object into a [`CongressBill`].
fn map_bill(item: &serde_json::Map<String, Value>) -> Option<CongressBill> {
    let congress = item
        .get("congress")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())?;
    let bill_type = item
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)?;
    // `number` arrives as either a JSON string or a JSON number; match those two
    // variants explicitly rather than `to_string()`-ing an arbitrary `Value`
    // (which would turn `null`/objects into the non-empty literals `"null"` etc.
    // and slip past the empty-string guard).
    let bill_number = item
        .get("number")
        .and_then(|value| match value {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Number(num) => Some(num.to_string()),
            _ => None,
        })
        .filter(|number| !number.is_empty())?;

    let latest = item.get("latestAction").and_then(Value::as_object);
    let sponsor = item
        .get("sponsors")
        .and_then(Value::as_array)
        .and_then(|sponsors| sponsors.first())
        .and_then(Value::as_object)
        .and_then(|sponsor| opt_string(sponsor.get("fullName")));
    let policy_area = item
        .get("policyArea")
        .and_then(Value::as_object)
        .and_then(|area| opt_string(area.get("name")));

    Some(CongressBill {
        congress,
        bill_type,
        bill_number,
        title: opt_string(item.get("title")),
        origin_chamber: opt_string(item.get("originChamber")),
        introduced_date: opt_string(item.get("introducedDate")),
        latest_action: latest.and_then(|action| opt_string(action.get("text"))),
        latest_action_date: latest.and_then(|action| opt_string(action.get("actionDate"))),
        update_date: opt_string(item.get("updateDate")),
        sponsor,
        policy_area,
        url: opt_string(item.get("url")),
    })
}

// ── Bills list fetcher ────────────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `congress.gov` bills-list fetcher.
    ///
    /// Standardizes `uscongress/bills` to [`tdw_domain::CongressBill`] rows from
    /// the `congress.gov` `/bill/{congress}/{billType}` list route.
    pub CongressGovBillsFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<CongressBillQuery, CongressBill> for CongressGovBillsFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "uscongress_bills";

    fn transform_query(params: Value) -> Result<CongressBillQuery> {
        CongressBillQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &CongressBillQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_congress(self.base_url(), query, "congress-gov bills").await
    }

    fn transform_data(&self, _query: &CongressBillQuery, raw: Bytes) -> Result<Vec<CongressBill>> {
        let object = decode_object(&raw, "congress-gov bills")?;
        let bills = object
            .get("bills")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::Provider("congress-gov bills parse_json: missing bills array".to_string())
            })?;
        Ok(bills
            .iter()
            .filter_map(Value::as_object)
            .filter_map(map_bill)
            .collect())
    }
}

// ── Bill detail fetcher ───────────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `congress.gov` single-bill detail fetcher.
    ///
    /// Standardizes `uscongress/bill_info` to a single
    /// [`tdw_domain::CongressBill`] row from the
    /// `/bill/{congress}/{billType}/{billNumber}` detail route.
    pub CongressGovBillInfoFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<CongressBillQuery, CongressBill> for CongressGovBillInfoFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "uscongress_bill_info";

    fn transform_query(params: Value) -> Result<CongressBillQuery> {
        CongressBillQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &CongressBillQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_congress(self.base_url(), query, "congress-gov bill_info").await
    }

    fn transform_data(&self, _query: &CongressBillQuery, raw: Bytes) -> Result<Vec<CongressBill>> {
        let object = decode_object(&raw, "congress-gov bill_info")?;
        let bill = object
            .get("bill")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                Error::Provider(
                    "congress-gov bill_info parse_json: missing bill object".to_string(),
                )
            })?;
        let row = map_bill(bill).ok_or_else(|| {
            Error::Provider(
                "congress-gov bill_info parse_json: bill missing identity fields".to_string(),
            )
        })?;
        Ok(vec![row])
    }
}

// ── Bill text-versions fetcher ────────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `congress.gov` bill text-version fetcher.
    ///
    /// Standardizes `uscongress/bill_text_urls` to [`tdw_domain::BillTextUrl`]
    /// rows (one per text-version/format pair) from the
    /// `/bill/{congress}/{billType}/{billNumber}/text` route.
    pub CongressGovBillTextFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<CongressBillQuery, BillTextUrl> for CongressGovBillTextFetcher {
    const PROVIDER: &'static str = crate::PROVIDER_ID;
    const ENDPOINT: &'static str = "uscongress_bill_text_urls";

    fn transform_query(params: Value) -> Result<CongressBillQuery> {
        CongressBillQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &CongressBillQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_congress(self.base_url(), query, "congress-gov bill_text_urls").await
    }

    fn transform_data(&self, _query: &CongressBillQuery, raw: Bytes) -> Result<Vec<BillTextUrl>> {
        let object = decode_object(&raw, "congress-gov bill_text_urls")?;
        let versions = object
            .get("textVersions")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::Provider(
                    "congress-gov bill_text_urls parse_json: missing textVersions array"
                        .to_string(),
                )
            })?;
        let mut rows = Vec::new();
        for version in versions.iter().filter_map(Value::as_object) {
            let version_type = opt_string(version.get("type"));
            let date = opt_string(version.get("date"));
            let Some(formats) = version.get("formats").and_then(Value::as_array) else {
                continue;
            };
            for format in formats.iter().filter_map(Value::as_object) {
                let (Some(format_type), Some(url)) = (
                    opt_string(format.get("type")),
                    opt_string(format.get("url")),
                ) else {
                    continue;
                };
                rows.push(BillTextUrl {
                    version_type: version_type.clone(),
                    date: date.clone(),
                    format_type,
                    url,
                });
            }
        }
        Ok(rows)
    }
}
