//! Real `Intrinio` v2 HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to the public `Intrinio` v2 REST
//! API (`https://api-v2.intrinio.com`). `Intrinio` is a PAID API: every request
//! carries the `api_key` query parameter read from `INTRINIO_API_KEY` via the
//! shared `tdw_core::http_support::read_required_key`. Live calls are gated by
//! `TDW_INTRINIO_LIVE=1` in the integration tests, which additionally skip when
//! the key is absent — the fetchers themselves compile and their
//! `transform_data` normalizers run offline over recorded fixtures.
//!
//! Clean-room: every route template, query parameter, and response field is
//! taken from `Intrinio`'s OWN public API docs (<https://docs.intrinio.com>).
//! `Intrinio` responses are JSON objects whose payload rows live under a named
//! array (e.g. `historical_data`, `trades`, `reported_financials`, `tags`); each
//! fetcher decodes that array and normalizes every row to a standardized
//! [`tdw_domain`] model.

#![cfg(feature = "http")]

use tdw_core::http_support::prelude::*;
use tdw_domain::{CompanyAttribute, Estimate, FinancialStatement, OptionContract, StatementKind};

use crate::{API_KEY_ENV, BASE_URL, IntrinioQuery};

const USER_AGENT: &str = "tdw-provider-intrinio/0.1";

// ── Shared HTTP / decode helpers ──────────────────────────────────────────────

/// Read the PAID `Intrinio` `api_key` from the environment.
fn api_key(ctx: &str) -> Result<String> {
    tdw_core::http_support::read_required_key(API_KEY_ENV, ctx)
}

/// Perform an `Intrinio` GET for `path` (appended to `base_url`) with `params`,
/// always attaching the required `api_key` query parameter, and return the raw
/// response bytes. The key is appended last so it is never logged via the static
/// `params` and cannot be overridden by a caller-supplied value.
async fn fetch_intrinio(
    base_url: &str,
    path: &str,
    params: &[(String, String)],
    ctx: &str,
) -> Result<Bytes> {
    let key = api_key(ctx)?;
    let endpoint = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let client = tdw_core::http_support::build_client(USER_AGENT, "intrinio http client build")?;
    let response = client
        .get(&endpoint)
        .query(params)
        .query(&[("api_key", key.as_str())])
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

/// Decode an `Intrinio` response body into the top-level JSON object.
fn decode_object(raw: &Bytes, ctx: &str) -> Result<serde_json::Map<String, Value>> {
    let value: Value = serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("{ctx} parse_json: {error}")))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(Error::Provider(format!(
            "{ctx} parse_json: expected a JSON object"
        ))),
    }
}

/// Extract the named array `key` from an `Intrinio` response object as a vector
/// of row objects. A missing / non-array field yields an empty vector (an empty
/// page is a valid response, not an error).
fn array_rows(
    map: &serde_json::Map<String, Value>,
    key: &str,
) -> Vec<serde_json::Map<String, Value>> {
    map.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_object().cloned())
                .collect()
        })
        .unwrap_or_default()
}

/// Read a numeric field, accepting a JSON number or a numeric string.
fn parse_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<f64>().ok()
            }
        }
        _ => None,
    }
}

/// Read a signed 32-bit integer field, accepting a JSON number or numeric
/// string (used for `fiscal_year`).
fn parse_i32(value: Option<&Value>) -> Option<i32> {
    parse_f64(value).and_then(|number| {
        if number.is_finite() {
            i32::try_from(number as i64).ok()
        } else {
            None
        }
    })
}

/// Read an unsigned integer field, accepting a JSON number or numeric string.
fn parse_u64(value: Option<&Value>) -> Option<u64> {
    parse_f64(value).and_then(|number| {
        if number.is_finite() && number >= 0.0 {
            Some(number as u64)
        } else {
            None
        }
    })
}

/// Read a non-empty string field, treating blank as absent.
fn parse_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Normalize an ISO-8601 timestamp (`"2024-06-04T00:00:00.000Z"`) or bare date
/// to the calendar `YYYY-MM-DD` form.
fn parse_date(value: Option<&Value>) -> Option<String> {
    let raw = parse_string(value)?;
    let date = raw.split(['T', ' ']).next().unwrap_or(raw.as_str());
    if date.is_empty() {
        None
    } else {
        Some(date.to_string())
    }
}

/// Resolve the option type to the standardized `"call"` / `"put"` label.
fn option_type(value: Option<&Value>) -> Option<String> {
    let raw = parse_string(value)?.to_ascii_lowercase();
    match raw.as_str() {
        "call" | "c" => Some("call".to_string()),
        "put" | "p" => Some("put".to_string()),
        _ => None,
    }
}

/// Build a standardized [`OptionContract`] from an `Intrinio` option row, given
/// the underlying symbol and the sub-objects the snapshot/unusual/chain shapes
/// share (`contract` metadata, optional `price`/`stats`/`greeks` blocks).
fn option_contract_from(
    underlying: &str,
    contract: &serde_json::Map<String, Value>,
    price: Option<&serde_json::Map<String, Value>>,
    greeks: Option<&serde_json::Map<String, Value>>,
) -> Option<OptionContract> {
    let expiration = parse_date(
        contract
            .get("expiration_date")
            .or_else(|| contract.get("date")),
    )?;
    let strike = parse_f64(contract.get("strike"))?;
    let option_type = option_type(contract.get("type").or_else(|| contract.get("option_type")))?;
    Some(OptionContract {
        underlying_symbol: underlying.to_string(),
        contract_symbol: parse_string(contract.get("code").or_else(|| contract.get("ticker"))),
        expiration,
        strike,
        option_type,
        bid: price.and_then(|block| parse_f64(block.get("bid"))),
        ask: price.and_then(|block| parse_f64(block.get("ask"))),
        last_price: price
            .and_then(|block| parse_f64(block.get("last").or_else(|| block.get("close")))),
        volume: price.and_then(|block| parse_u64(block.get("volume"))),
        open_interest: price.and_then(|block| parse_u64(block.get("open_interest"))),
        implied_volatility: greeks
            .and_then(|block| parse_f64(block.get("implied_volatility")))
            .or_else(|| price.and_then(|block| parse_f64(block.get("implied_volatility")))),
        delta: greeks.and_then(|block| parse_f64(block.get("delta"))),
        gamma: greeks.and_then(|block| parse_f64(block.get("gamma"))),
        theta: greeks.and_then(|block| parse_f64(block.get("theta"))),
        vega: greeks.and_then(|block| parse_f64(block.get("vega"))),
        rho: greeks.and_then(|block| parse_f64(block.get("rho"))),
    })
}

/// Require the query's `identifier`, erroring with a route-named message.
fn required_identifier<'a>(query: &'a IntrinioQuery, ctx: &str) -> Result<&'a str> {
    query
        .identifier
        .as_deref()
        .ok_or_else(|| Error::Provider(format!("{ctx} requires an identifier")))
}

/// Require the query's `tag`, erroring with a route-named message.
fn required_tag<'a>(query: &'a IntrinioQuery, ctx: &str) -> Result<&'a str> {
    query
        .tag
        .as_deref()
        .ok_or_else(|| Error::Provider(format!("{ctx} requires a tag")))
}

/// Append the standard `start_date`/`end_date`/`limit` params (page-size style)
/// to an `Intrinio` request when the caller supplied them.
fn push_standard_params(query: &IntrinioQuery, params: &mut Vec<(String, String)>) {
    if let Some(start) = &query.params.start_date {
        params.push(("start_date".to_string(), start.to_string()));
    }
    if let Some(end) = &query.params.end_date {
        params.push(("end_date".to_string(), end.to_string()));
    }
    if let Some(limit) = query.params.limit {
        params.push(("page_size".to_string(), limit.to_string()));
    }
}

// ── equity/fundamental/historical_attributes ─────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` historical-attributes fetcher.
    ///
    /// Standardizes `equity/fundamental/historical_attributes` to
    /// [`tdw_domain::CompanyAttribute`] rows from the `Intrinio`
    /// `/historical_data/{identifier}/{tag}` time-series route.
    pub IntrinioHttpHistoricalAttributesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, CompanyAttribute> for IntrinioHttpHistoricalAttributesFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "equity_fundamental_historical_attributes";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio historical_attributes";
        let identifier = required_identifier(query, ctx)?;
        let tag = required_tag(query, ctx)?;
        let path = format!("historical_data/{identifier}/{tag}");
        let mut params = Vec::new();
        push_standard_params(query, &mut params);
        fetch_intrinio(self.base_url(), &path, &params, ctx).await
    }

    fn transform_data(&self, query: &IntrinioQuery, raw: Bytes) -> Result<Vec<CompanyAttribute>> {
        let map = decode_object(&raw, "intrinio historical_attributes")?;
        let symbol = query.identifier.clone();
        let tag = query.tag.clone().unwrap_or_default();
        let rows = array_rows(&map, "historical_data");
        let mut out = Vec::with_capacity(rows.len());
        for record in rows {
            out.push(CompanyAttribute {
                symbol: symbol.clone(),
                tag: tag.clone(),
                name: None,
                date: parse_date(record.get("date")),
                value: parse_f64(record.get("value")),
                text_value: None,
                unit: None,
                statement_code: None,
            });
        }
        Ok(out)
    }
}

// ── equity/fundamental/latest_attributes ─────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` latest-attribute fetcher.
    ///
    /// Standardizes `equity/fundamental/latest_attributes` to a single
    /// [`tdw_domain::CompanyAttribute`] from the `Intrinio`
    /// `/companies/{identifier}/data_point/{tag}` data-point route.
    pub IntrinioHttpLatestAttributesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, CompanyAttribute> for IntrinioHttpLatestAttributesFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "equity_fundamental_latest_attributes";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio latest_attributes";
        let identifier = required_identifier(query, ctx)?;
        let tag = required_tag(query, ctx)?;
        // `data_point/{tag}/number` returns a typed `{ "value": ... }` envelope.
        let path = format!("companies/{identifier}/data_point/{tag}");
        fetch_intrinio(self.base_url(), &path, &[], ctx).await
    }

    fn transform_data(&self, query: &IntrinioQuery, raw: Bytes) -> Result<Vec<CompanyAttribute>> {
        let map = decode_object(&raw, "intrinio latest_attributes")?;
        let tag = query.tag.clone().unwrap_or_default();
        let value = parse_f64(map.get("value"));
        let text_value = if value.is_none() {
            parse_string(map.get("value"))
        } else {
            None
        };
        Ok(vec![CompanyAttribute {
            symbol: query.identifier.clone(),
            tag,
            name: None,
            date: parse_date(map.get("date").or_else(|| map.get("last_updated"))),
            value,
            text_value,
            unit: None,
            statement_code: None,
        }])
    }
}

// ── equity/fundamental/search_attributes ─────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` data-tag search fetcher.
    ///
    /// Standardizes `equity/fundamental/search_attributes` to
    /// [`tdw_domain::CompanyAttribute`] metadata rows from the `Intrinio`
    /// `/data_tags/search` dictionary route (one row per matched tag).
    pub IntrinioHttpSearchAttributesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, CompanyAttribute> for IntrinioHttpSearchAttributesFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "equity_fundamental_search_attributes";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio search_attributes";
        let search = required_tag(query, ctx)?;
        let mut params = vec![("query".to_string(), search.to_string())];
        if let Some(limit) = query.params.limit {
            params.push(("page_size".to_string(), limit.to_string()));
        }
        fetch_intrinio(self.base_url(), "data_tags/search", &params, ctx).await
    }

    fn transform_data(&self, _query: &IntrinioQuery, raw: Bytes) -> Result<Vec<CompanyAttribute>> {
        let map = decode_object(&raw, "intrinio search_attributes")?;
        let rows = array_rows(&map, "tags");
        let mut out = Vec::with_capacity(rows.len());
        for record in rows {
            let Some(tag) = parse_string(record.get("tag")) else {
                continue;
            };
            out.push(CompanyAttribute {
                symbol: None,
                tag,
                name: parse_string(record.get("name")),
                date: None,
                value: None,
                text_value: None,
                unit: parse_string(record.get("unit").or_else(|| record.get("type"))),
                statement_code: parse_string(record.get("statement_code")),
            });
        }
        Ok(out)
    }
}

// ── equity/fundamental/reported_financials ───────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` as-reported financials fetcher.
    ///
    /// Standardizes `equity/fundamental/reported_financials` to
    /// [`tdw_domain::FinancialStatement`] from the `Intrinio`
    /// `/fundamentals/{id}/reported_financials` route, collapsing the wide
    /// line-item list into the model's `line_items` bag.
    pub IntrinioHttpReportedFinancialsFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, FinancialStatement> for IntrinioHttpReportedFinancialsFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "equity_fundamental_reported_financials";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio reported_financials";
        let fundamental_id = required_identifier(query, ctx)?;
        let path = format!("fundamentals/{fundamental_id}/reported_financials");
        fetch_intrinio(self.base_url(), &path, &[], ctx).await
    }

    fn transform_data(&self, query: &IntrinioQuery, raw: Bytes) -> Result<Vec<FinancialStatement>> {
        let map = decode_object(&raw, "intrinio reported_financials")?;
        let rows = array_rows(&map, "reported_financials");
        let mut line_items = std::collections::BTreeMap::new();
        for record in &rows {
            // Each row carries an `xbrl_tag` (or nested `reported_tag.tag`) and a
            // numeric `value`; collapse the wide list into the model's bag.
            let tag = parse_string(record.get("xbrl_tag")).or_else(|| {
                record
                    .get("reported_tag")
                    .and_then(Value::as_object)
                    .and_then(|nested| parse_string(nested.get("tag")))
            });
            if let (Some(tag), Some(value)) = (tag, parse_f64(record.get("value"))) {
                line_items.insert(tag, value);
            }
        }
        if line_items.is_empty() {
            return Ok(Vec::new());
        }
        // The response carries a sibling `fundamental` object describing the
        // statement's company and period. Prefer its ticker for the `symbol`
        // anchor and lift the typed period headers from it, falling back to the
        // fundamental id from the query when the block is absent.
        let fundamental = map.get("fundamental").and_then(Value::as_object);
        let symbol = fundamental
            .and_then(|fund| fund.get("company"))
            .and_then(Value::as_object)
            .and_then(|company| parse_string(company.get("ticker")))
            .or_else(|| query.identifier.clone())
            .unwrap_or_default();
        let fiscal_year = fundamental.and_then(|fund| parse_i32(fund.get("fiscal_year")));
        let fiscal_period = fundamental.and_then(|fund| parse_string(fund.get("fiscal_period")));
        let date = fundamental.and_then(|fund| parse_date(fund.get("end_date")));
        let filing_date = fundamental.and_then(|fund| parse_date(fund.get("filing_date")));
        Ok(vec![FinancialStatement {
            symbol,
            statement: StatementKind::Income,
            period: "reported".to_string(),
            fiscal_year,
            fiscal_period,
            date,
            filing_date,
            currency: None,
            line_items,
        }])
    }
}

// ── equity/estimates/forward_pe & forward_sales ──────────────────────────────

/// Shared transform for the two forward-estimate routes: one `Estimate` row per
/// fiscal period, tagged by `kind`, reading the `value`/`mean`/`low`/`high`
/// fields the `Intrinio` zacks forward-estimate shapes share.
fn forward_estimates(
    map: &serde_json::Map<String, Value>,
    array_key: &str,
    symbol: &str,
    kind: &str,
) -> Vec<Estimate> {
    let rows = array_rows(map, array_key);
    let mut out = Vec::with_capacity(rows.len());
    for record in rows {
        out.push(Estimate {
            symbol: symbol.to_string(),
            kind: kind.to_string(),
            fiscal_period: parse_string(
                record
                    .get("fiscal_year")
                    .or_else(|| record.get("calendar_year"))
                    .or_else(|| record.get("fiscal_period")),
            ),
            date: parse_date(record.get("date").or_else(|| record.get("updated_date"))),
            analyst: None,
            recommendation: None,
            value: parse_f64(record.get("value").or_else(|| record.get("mean"))),
            low: parse_f64(record.get("low")),
            high: parse_f64(record.get("high")),
            mean: parse_f64(record.get("mean")),
            number_of_analysts: parse_u64(
                record.get("count").or_else(|| record.get("analyst_count")),
            )
            .and_then(|count| u32::try_from(count).ok()),
            currency: parse_string(record.get("currency")),
        });
    }
    out
}

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` forward-PE estimates fetcher.
    ///
    /// Standardizes `equity/estimates/forward_pe` to [`tdw_domain::Estimate`]
    /// rows (`kind = "forward_pe"`) from the `Intrinio` zacks forward-PE route.
    pub IntrinioHttpForwardPeFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, Estimate> for IntrinioHttpForwardPeFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "equity_estimates_forward_pe";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio forward_pe";
        let identifier = required_identifier(query, ctx)?;
        let path = format!("zacks/forward_pe/{identifier}");
        fetch_intrinio(self.base_url(), &path, &[], ctx).await
    }

    fn transform_data(&self, query: &IntrinioQuery, raw: Bytes) -> Result<Vec<Estimate>> {
        let map = decode_object(&raw, "intrinio forward_pe")?;
        let symbol = query.identifier.clone().unwrap_or_default();
        Ok(forward_estimates(
            &map,
            "forward_pe_estimates",
            &symbol,
            "forward_pe",
        ))
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` forward-sales estimates fetcher.
    ///
    /// Standardizes `equity/estimates/forward_sales` to [`tdw_domain::Estimate`]
    /// rows (`kind = "forward_sales"`) from the `Intrinio` zacks forward-sales
    /// route.
    pub IntrinioHttpForwardSalesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, Estimate> for IntrinioHttpForwardSalesFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "equity_estimates_forward_sales";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio forward_sales";
        let identifier = required_identifier(query, ctx)?;
        let path = format!("zacks/forward_sales/{identifier}");
        fetch_intrinio(self.base_url(), &path, &[], ctx).await
    }

    fn transform_data(&self, query: &IntrinioQuery, raw: Bytes) -> Result<Vec<Estimate>> {
        let map = decode_object(&raw, "intrinio forward_sales")?;
        let symbol = query.identifier.clone().unwrap_or_default();
        Ok(forward_estimates(
            &map,
            "forward_sales_estimates",
            &symbol,
            "forward_sales",
        ))
    }
}

// ── derivatives/options/unusual ──────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` unusual-options-activity fetcher.
    ///
    /// Standardizes `derivatives/options/unusual` to [`tdw_domain::OptionContract`]
    /// rows from the `Intrinio` `/options/unusual_activity/{symbol}` route. Each
    /// trade carries the contract metadata plus the executed price/volume.
    pub IntrinioHttpOptionsUnusualFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, OptionContract> for IntrinioHttpOptionsUnusualFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "derivatives_options_unusual";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio options_unusual";
        let symbol = required_identifier(query, ctx)?;
        let path = format!("options/unusual_activity/{symbol}");
        let mut params = Vec::new();
        push_standard_params(query, &mut params);
        fetch_intrinio(self.base_url(), &path, &params, ctx).await
    }

    fn transform_data(&self, query: &IntrinioQuery, raw: Bytes) -> Result<Vec<OptionContract>> {
        let map = decode_object(&raw, "intrinio options_unusual")?;
        let underlying = query.identifier.clone().unwrap_or_default();
        let rows = array_rows(&map, "trades");
        let mut out = Vec::with_capacity(rows.len());
        for record in rows {
            // The trade row carries contract identity inline plus the executed
            // `total_value`/`total_size`; map them through the shared builder.
            if let Some(contract) = option_contract_from(&underlying, &record, Some(&record), None)
            {
                out.push(contract);
            }
        }
        Ok(out)
    }
}

// ── derivatives/options/snapshots ────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` options-snapshots fetcher.
    ///
    /// Standardizes `derivatives/options/snapshots` to [`tdw_domain::OptionContract`]
    /// rows from the `Intrinio` `/options/snapshots` route, reading the per-row
    /// `contract`, `price`, and `greeks` blocks.
    pub IntrinioHttpOptionsSnapshotsFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, OptionContract> for IntrinioHttpOptionsSnapshotsFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "derivatives_options_snapshots";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio options_snapshots";
        let mut params = Vec::new();
        push_standard_params(query, &mut params);
        fetch_intrinio(self.base_url(), "options/snapshots", &params, ctx).await
    }

    fn transform_data(&self, query: &IntrinioQuery, raw: Bytes) -> Result<Vec<OptionContract>> {
        let map = decode_object(&raw, "intrinio options_snapshots")?;
        let fallback_underlying = query.identifier.clone().unwrap_or_default();
        let rows = array_rows(&map, "contracts");
        let mut out = Vec::with_capacity(rows.len());
        for record in rows {
            let contract = record
                .get("contract")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_else(|| record.clone());
            let price = record.get("price").and_then(Value::as_object).cloned();
            let greeks = record.get("greeks").and_then(Value::as_object).cloned();
            let underlying = parse_string(contract.get("underlying"))
                .or_else(|| parse_string(record.get("underlying")))
                .unwrap_or_else(|| fallback_underlying.clone());
            if let Some(option) =
                option_contract_from(&underlying, &contract, price.as_ref(), greeks.as_ref())
            {
                out.push(option);
            }
        }
        Ok(out)
    }
}

// ── derivatives/options/surface ──────────────────────────────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production `Intrinio` options-surface (IV-input) fetcher.
    ///
    /// Standardizes `derivatives/options/surface` to [`tdw_domain::OptionContract`]
    /// rows from the `Intrinio` `/options/chain/{symbol}/{expiration}` route,
    /// carrying the per-contract implied-volatility and greeks **inputs**. The
    /// full IV-surface solver (interpolated vol grid) is a documented follow-up:
    /// this fetcher ships the chain + IV inputs rather than fabricating a surface.
    pub IntrinioHttpOptionsSurfaceFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<IntrinioQuery, OptionContract> for IntrinioHttpOptionsSurfaceFetcher {
    const PROVIDER: &'static str = "intrinio";
    const ENDPOINT: &'static str = "derivatives_options_surface";

    fn transform_query(params: Value) -> Result<IntrinioQuery> {
        IntrinioQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &IntrinioQuery, _creds: &Credentials) -> Result<Bytes> {
        let ctx = "intrinio options_surface";
        let symbol = required_identifier(query, ctx)?;
        // `tag` carries the expiration for the chain slice; default to the
        // provider's `eod` keyword (a documented sentinel for the latest chain).
        let expiration = query.tag.as_deref().unwrap_or("eod");
        let path = format!("options/chain/{symbol}/{expiration}");
        fetch_intrinio(self.base_url(), &path, &[], ctx).await
    }

    fn transform_data(&self, query: &IntrinioQuery, raw: Bytes) -> Result<Vec<OptionContract>> {
        let map = decode_object(&raw, "intrinio options_surface")?;
        let underlying = query.identifier.clone().unwrap_or_default();
        let rows = array_rows(&map, "chain");
        let mut out = Vec::with_capacity(rows.len());
        for record in rows {
            let contract = record
                .get("option")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_else(|| record.clone());
            let price = record.get("price").and_then(Value::as_object).cloned();
            if let Some(option) =
                option_contract_from(&underlying, &contract, price.as_ref(), price.as_ref())
            {
                out.push(option);
            }
        }
        Ok(out)
    }
}
