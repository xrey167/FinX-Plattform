#![cfg(feature = "http")]
//! Real ECB Statistical Data Warehouse HTTP fetcher.
//!
//! Gated by the `http` feature. Talks to the ECB SDW REST API
//! `/data/{flow}/{key}` endpoint via `reqwest`. No authentication is required.
//! The live integration test is additionally gated by `TDW_ECB_LIVE=1` so
//! unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{BASE_URL, EcbDataQuery, EcbObservation, parse_ecb_value};

const USER_AGENT: &str = "tdw-provider-ecb/0.1";

/// Provider specification for the ECB SDW data fetcher.
pub struct EcbDataSpec;

impl ProviderSpec for EcbDataSpec {
    const PROVIDER: &'static str = "ecb";
    const ENDPOINT: &'static str = "data";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "ecb client build";
    const SEND_ERR: &'static str = "ecb extract_data";
    const RETURNED_ERR: &'static str = "ecb extract_data returned";
    const READ_BODY_ERR: &'static str = "ecb read body";

    type Query = EcbDataQuery;
    type Data = EcbObservation;

    fn transform_query(params: Value) -> Result<EcbDataQuery> {
        let flow = params
            .get("flow")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb flow must be a string".to_string()))?;
        let key = params
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb key must be a string".to_string()))?;
        let start_period = params
            .get("start_period")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb start_period must be a string".to_string()))?;
        let end_period = params
            .get("end_period")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("ecb end_period must be a string".to_string()))?;
        EcbDataQuery::new(flow, key, start_period, end_period)
            .map_err(|error| Error::InvalidQuery(error.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &EcbDataQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let endpoint = format!(
            "{}/data/{}/{}",
            base_url.trim_end_matches('/'),
            query.flow,
            query.key,
        );
        let query_params = [
            ("format", "jsondata"),
            ("startPeriod", query.start_period.as_str()),
            ("endPeriod", query.end_period.as_str()),
        ];
        Ok(client.get(&endpoint).query(&query_params))
    }

    fn transform_data(query: &EcbDataQuery, raw: Bytes) -> Result<Vec<EcbObservation>> {
        let v: Value = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("ecb parse_json: {error}")))?;
        Ok(parse_ecb_value(&v, &query.flow, &query.key))
    }
}

/// Production ECB SDW data fetcher.
pub type EcbHttpDataFetcher = HttpFetcher<EcbDataSpec>;

// ---------------------------------------------------------------------------
// Catalog-facing reference-rates fetcher -> tdw_domain::MacroSeries
//
// The bespoke `EcbHttpDataFetcher` above emits crate-local `EcbObservation`s.
// The namespaced endpoint catalog (`currency/reference_rates`) needs a fetcher
// that emits a `tdw_domain` model so a route can be dispatched without any
// crate-local row shape leaking into the spine. This thin wrapper reuses the
// shared `parse_ecb_value` SDMX/`jsondata` parser and the `data_request_path`
// guards verbatim, mapping each `(date, currency, value)` onto a
// `MacroSeries` row.
//
// The EXR daily euro foreign-exchange reference rates (flow `EXR`, key
// `D.<CCY>.EUR.SP00.A`) are the canonical "reference rates" snapshot; a blank
// currency dimension (`D..EUR.SP00.A`) selects every published pair in one
// request. See the ECB SDW REST API and the EXR dataflow:
// <https://data-api.ecb.europa.eu/service/data/EXR/D..EUR.SP00.A>.
// ---------------------------------------------------------------------------

use tdw_core::query_params::StandardParams;
use tdw_domain::MacroSeries;

use crate::{data_request_path, parse_ecb_reference_rates};

/// EXR dataflow identifier for the daily euro FX reference rates.
const REFERENCE_RATES_FLOW: &str = "EXR";
/// EXR series key selecting every published euro reference pair (the currency
/// dimension is left blank, an SDMX wildcard).
const REFERENCE_RATES_KEY: &str = "D..EUR.SP00.A";

/// Validated query for the standardized `currency/reference_rates` route.
///
/// Carries the shared `start_date`/`end_date` window (mapped onto the ECB SDW
/// `startPeriod`/`endPeriod` request parameters); the flow and series key are
/// fixed by the route definition, not the caller. When no window is supplied the
/// fetcher requests only the latest available reference snapshot.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct EcbReferenceRatesQuery {
    /// Shared date normalization, applied to the SDW request window.
    #[serde(default, flatten)]
    pub params: StandardParams,
}

impl EcbReferenceRatesQuery {
    /// Build a reference-rates query from a raw caller payload.
    ///
    /// # Errors
    ///
    /// Returns [`tdw_core::Error::InvalidQuery`] when the shared parameters fail
    /// validation (e.g. an inverted date range).
    pub fn from_value(params: &Value) -> Result<Self> {
        Ok(Self {
            params: StandardParams::from_value(params)?,
        })
    }
}

tdw_core::provider_fetcher_struct!(
    /// Production ECB euro FX reference-rates fetcher.
    ///
    /// Standardizes `currency/reference_rates`: the daily euro foreign-exchange
    /// reference rates (EXR flow), normalized to [`tdw_domain::MacroSeries`] —
    /// one row per `(currency pair, date)`. Reuses the shared SDMX `jsondata`
    /// parser; no parsing logic is duplicated.
    pub EcbHttpReferenceRatesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<EcbReferenceRatesQuery, MacroSeries> for EcbHttpReferenceRatesFetcher {
    const PROVIDER: &'static str = "ecb";
    const ENDPOINT: &'static str = "reference_rates";

    fn transform_query(params: Value) -> Result<EcbReferenceRatesQuery> {
        EcbReferenceRatesQuery::from_value(&params)
    }

    async fn extract_data(
        &self,
        query: &EcbReferenceRatesQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        // Default to the most recent published day when the caller gives no
        // window; the ECB SDW returns the latest observation when no
        // startPeriod/endPeriod is supplied, so we map the optional window
        // straight through.
        let start = query
            .params
            .start_date
            .map_or_else(String::new, |date| date.to_string());
        let end = query
            .params
            .end_date
            .map_or_else(String::new, |date| date.to_string());
        // Reuse the crate's path guard for its SDMX-grammar validation.
        data_request_path(REFERENCE_RATES_FLOW, REFERENCE_RATES_KEY, &start, &end)
            .map_err(|error| Error::Provider(error.to_string()))?;

        let endpoint = format!(
            "{}/data/{REFERENCE_RATES_FLOW}/{REFERENCE_RATES_KEY}",
            self.base_url().trim_end_matches('/'),
        );
        let mut query_params: Vec<(&str, String)> = vec![("format", "jsondata".to_string())];
        if !start.is_empty() {
            query_params.push(("startPeriod", start));
        }
        if !end.is_empty() {
            query_params.push(("endPeriod", end));
        }
        let client = tdw_core::http_support::build_client(USER_AGENT, "ecb client build")?;
        let response = client
            .get(&endpoint)
            .query(&query_params)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("ecb extract_data: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "ecb extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("ecb read body: {error}")))
    }

    fn transform_data(
        &self,
        _query: &EcbReferenceRatesQuery,
        raw: Bytes,
    ) -> Result<Vec<MacroSeries>> {
        // Reuse the shared SDMX parser, which resolves each series' CURRENCY
        // dimension so a wildcard request yields one labelled row per pair.
        let rates = parse_ecb_reference_rates(&raw)
            .map_err(|error| Error::Provider(format!("ecb parse_json: {error}")))?;
        Ok(rates
            .into_iter()
            .map(|rate| MacroSeries {
                series_id: rate.currency,
                title: Some("ECB euro foreign-exchange reference rate".to_string()),
                date: rate.date,
                value: Some(rate.value),
                country: None,
                frequency: Some("daily".to_string()),
                unit: Some("eur_per_unit".to_string()),
                transform: None,
            })
            .collect())
    }
}
