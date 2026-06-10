#![cfg(feature = "http")]
//! Real EIA HTTP fetchers for spot-price and natural-gas endpoints.
//!
//! Both fetchers implement the canonical [`tdw_core::Fetcher`] trait and are
//! gated by the `http` Cargo feature. Live calls require `TDW_EIA_API_KEY`;
//! the live integration tests are additionally gated by `TDW_EIA_LIVE=1` so
//! unattended CI stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use tdw_core::{Error, Result};
use tdw_provider_http::{HttpFetcher, ProviderSpec};

use crate::{
    API_KEY_ENV, BASE_URL, EiaNaturalGasQuery, EiaNaturalGasRecord, EiaSpotPriceQuery,
    EiaSpotPriceRecord, PROVIDER_ID,
};

const USER_AGENT: &str = "tdw-provider-eia/0.1";

// ---------------------------------------------------------------------------
// Serde shapes for /petroleum/pri/spt/data/
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EiaSpotPriceEnvelope {
    response: EiaSpotPriceResponse,
}

#[derive(Deserialize)]
struct EiaSpotPriceResponse {
    #[serde(default)]
    data: Vec<EiaSpotPriceRaw>,
}

#[derive(Deserialize)]
struct EiaSpotPriceRaw {
    #[serde(default)]
    period: String,
    #[serde(rename = "product-name", default)]
    product_name: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    units: String,
}

// ---------------------------------------------------------------------------
// Serde shapes for /natural-gas/pri/sum/data/
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EiaNaturalGasEnvelope {
    response: EiaNaturalGasResponse,
}

#[derive(Deserialize)]
struct EiaNaturalGasResponse {
    #[serde(default)]
    data: Vec<EiaNaturalGasRaw>,
}

#[derive(Deserialize)]
struct EiaNaturalGasRaw {
    #[serde(default)]
    period: String,
    #[serde(rename = "series-description", default)]
    series_description: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    units: String,
}

// ---------------------------------------------------------------------------
// EiaHttpSpotPriceFetcher
// ---------------------------------------------------------------------------

/// Provider specification for the EIA petroleum spot-price fetcher.
///
/// Calls `GET /petroleum/pri/spt/data/` with daily frequency.
pub struct EiaSpotPriceSpec;

impl ProviderSpec for EiaSpotPriceSpec {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "spot_price";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "eia build client";
    const SEND_ERR: &'static str = "eia spot-price request";
    const RETURNED_ERR: &'static str = "eia spot-price returned";
    const READ_BODY_ERR: &'static str = "eia spot-price read body";
    const UNREADABLE_BODY: Option<&'static str> = Some("<unreadable body>");

    type Query = EiaSpotPriceQuery;
    type Data = EiaSpotPriceRecord;

    fn transform_query(params: Value) -> Result<EiaSpotPriceQuery> {
        let query: EiaSpotPriceQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("eia spot-price query: {e}")))?;
        EiaSpotPriceQuery::new(query.commodity, query.length)
            .map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &EiaSpotPriceQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "eia")?;
        let endpoint = format!("{}/petroleum/pri/spt/data/", base_url.trim_end_matches('/'));
        let params = [
            ("api_key", api_key),
            ("frequency", "daily".to_string()),
            ("data[0]", "value".to_string()),
            ("sort[0][column]", "period".to_string()),
            ("sort[0][direction]", "desc".to_string()),
            ("length", query.length.to_string()),
        ];
        Ok(client.get(&endpoint).query(&params))
    }

    fn transform_data(_query: &EiaSpotPriceQuery, raw: Bytes) -> Result<Vec<EiaSpotPriceRecord>> {
        let envelope: EiaSpotPriceEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("eia spot-price parse_json: {e}")))?;
        let mut records = Vec::with_capacity(envelope.response.data.len());
        for row in envelope.response.data {
            if row.period.is_empty() {
                return Err(Error::Provider(
                    "eia spot-price row missing period".to_string(),
                ));
            }
            let raw_value = row.value.trim();
            if raw_value.is_empty() || raw_value == "." {
                continue;
            }
            let value = raw_value.parse::<f64>().map_err(|e| {
                Error::Provider(format!(
                    "eia spot-price value parse failed for {}: {e}",
                    row.period
                ))
            })?;
            records.push(EiaSpotPriceRecord {
                period: row.period,
                product_name: row.product_name,
                value,
                units: row.units,
            });
        }
        Ok(records)
    }
}

/// Production EIA petroleum spot-price fetcher.
pub type EiaHttpSpotPriceFetcher = HttpFetcher<EiaSpotPriceSpec>;

// ---------------------------------------------------------------------------
// EiaHttpNaturalGasFetcher
// ---------------------------------------------------------------------------

/// Provider specification for the EIA natural-gas price fetcher.
///
/// Calls `GET /natural-gas/pri/sum/data/` with monthly frequency.
pub struct EiaNaturalGasSpec;

impl ProviderSpec for EiaNaturalGasSpec {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "natural_gas";
    const USER_AGENT: &'static str = USER_AGENT;
    const DEFAULT_BASE_URL: &'static str = BASE_URL;

    const CLIENT_ERR: &'static str = "eia build client";
    const SEND_ERR: &'static str = "eia natural-gas request";
    const RETURNED_ERR: &'static str = "eia natural-gas returned";
    const READ_BODY_ERR: &'static str = "eia natural-gas read body";
    const UNREADABLE_BODY: Option<&'static str> = Some("<unreadable body>");

    type Query = EiaNaturalGasQuery;
    type Data = EiaNaturalGasRecord;

    fn transform_query(params: Value) -> Result<EiaNaturalGasQuery> {
        let query: EiaNaturalGasQuery = serde_json::from_value(params)
            .map_err(|e| Error::InvalidQuery(format!("eia natural-gas query: {e}")))?;
        EiaNaturalGasQuery::new(query.length).map_err(|e| Error::InvalidQuery(e.to_string()))
    }

    fn build_request(
        base_url: &str,
        query: &EiaNaturalGasQuery,
        client: &Client,
    ) -> Result<reqwest::RequestBuilder> {
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "eia")?;
        let endpoint = format!(
            "{}/natural-gas/pri/sum/data/",
            base_url.trim_end_matches('/')
        );
        let params = [
            ("api_key", api_key),
            ("frequency", "monthly".to_string()),
            ("data[0]", "value".to_string()),
            ("length", query.length.to_string()),
        ];
        Ok(client.get(&endpoint).query(&params))
    }

    fn transform_data(_query: &EiaNaturalGasQuery, raw: Bytes) -> Result<Vec<EiaNaturalGasRecord>> {
        let envelope: EiaNaturalGasEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("eia natural-gas parse_json: {e}")))?;
        let mut records = Vec::with_capacity(envelope.response.data.len());
        for row in envelope.response.data {
            if row.period.is_empty() {
                return Err(Error::Provider(
                    "eia natural-gas row missing period".to_string(),
                ));
            }
            let raw_value = row.value.trim();
            if raw_value.is_empty() || raw_value == "." {
                continue;
            }
            let value = raw_value.parse::<f64>().map_err(|e| {
                Error::Provider(format!(
                    "eia natural-gas value parse failed for {}: {e}",
                    row.period
                ))
            })?;
            records.push(EiaNaturalGasRecord {
                period: row.period,
                series_description: row.series_description,
                value,
                units: row.units,
            });
        }
        Ok(records)
    }
}

/// Production EIA natural-gas price fetcher.
pub type EiaHttpNaturalGasFetcher = HttpFetcher<EiaNaturalGasSpec>;

// ---------------------------------------------------------------------------
// Catalog-facing report fetcher -> tdw_domain::CommodityReportRow
//
// The bespoke spot-price/natural-gas fetchers above emit crate-local row
// shapes. The namespaced endpoint catalog routes
// `commodity/petroleum_status_report` (the Weekly Petroleum Status Report) and
// `commodity/short_term_energy_outlook` (STEO) need a fetcher that emits a
// `tdw_domain` model. This wrapper reuses the EIA v2 `/data/` request contract
// (api-key guard + `data[0]=value` + sort) and the shared string-value parsing,
// mapping each row onto `CommodityReportRow`. One fetcher serves both reports;
// the report discriminator is injected per dispatch binding, mirroring the
// FRED command-injection pattern.
//
// EIA v2 data API: <https://www.eia.gov/opendata/documentation.php>. WPSR
// weekly crude/product stocks: route `petroleum/stoc/wstk`. STEO monthly
// outlook series: route `steo`.
// ---------------------------------------------------------------------------

use tdw_core::{Credentials, Fetcher};
use tdw_domain::CommodityReportRow;

/// Which EIA report a [`EiaReportQuery`] targets.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EiaReport {
    /// Weekly Petroleum Status Report (weekly crude/product stocks).
    PetroleumStatusReport,
    /// Short-Term Energy Outlook (monthly outlook series).
    ShortTermEnergyOutlook,
}

impl EiaReport {
    /// The EIA v2 data-route path segment under the API base.
    const fn route(self) -> &'static str {
        match self {
            Self::PetroleumStatusReport => "petroleum/stoc/wstk",
            Self::ShortTermEnergyOutlook => "steo",
        }
    }

    /// The EIA v2 frequency for the report's series.
    const fn frequency(self) -> &'static str {
        match self {
            Self::PetroleumStatusReport => "weekly",
            Self::ShortTermEnergyOutlook => "monthly",
        }
    }

    /// The canonical report identifier carried on each emitted row. This is also
    /// the `snake_case` serde value, so the dispatcher can inject it as the
    /// `report` query field for the per-report dispatch binding.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::PetroleumStatusReport => "petroleum_status_report",
            Self::ShortTermEnergyOutlook => "short_term_energy_outlook",
        }
    }
}

/// Validated query for the standardized EIA report routes.
///
/// Carries the report discriminator (injected per dispatch binding) plus the
/// shared `length` cap reused from [`crate::EiaNaturalGasQuery`]'s validation.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct EiaReportQuery {
    /// Which report to fetch.
    pub report: EiaReport,
    /// Maximum number of rows to request (validated `1..=10_000`).
    #[serde(default = "default_report_length")]
    pub length: u32,
}

const fn default_report_length() -> u32 {
    100
}

impl EiaReportQuery {
    /// Build a report query from a raw caller payload.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidQuery`] when `report` is missing/invalid or the
    /// `length` falls outside `1..=10_000`.
    pub fn from_value(params: &Value) -> Result<Self> {
        let query: Self = serde_json::from_value(params.clone())
            .map_err(|e| Error::InvalidQuery(format!("eia report query: {e}")))?;
        crate::EiaSpotPriceQuery::new(crate::EiaCommodity::CrudeOilWti, query.length)
            .map_err(|e| Error::InvalidQuery(e.to_string()))?;
        Ok(query)
    }
}

#[derive(Deserialize)]
struct EiaReportEnvelope {
    response: EiaReportResponse,
}

#[derive(Deserialize)]
struct EiaReportResponse {
    #[serde(default)]
    data: Vec<EiaReportRaw>,
}

#[derive(Deserialize)]
struct EiaReportRaw {
    #[serde(default)]
    period: String,
    #[serde(rename = "series", default)]
    series: String,
    #[serde(rename = "series-description", default)]
    series_description: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    units: String,
}

tdw_core::provider_fetcher_struct!(
    /// Production EIA report fetcher (catalog-facing).
    ///
    /// Standardizes `commodity/petroleum_status_report` and
    /// `commodity/short_term_energy_outlook`, normalized to
    /// [`tdw_domain::CommodityReportRow`]. Reuses the EIA v2 `/data/` request
    /// contract and the shared string-value parsing.
    pub EiaHttpReportFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<EiaReportQuery, CommodityReportRow> for EiaHttpReportFetcher {
    const PROVIDER: &'static str = PROVIDER_ID;
    const ENDPOINT: &'static str = "report";

    fn transform_query(params: Value) -> Result<EiaReportQuery> {
        EiaReportQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &EiaReportQuery, _creds: &Credentials) -> Result<Bytes> {
        let api_key = tdw_core::http_support::read_required_key(API_KEY_ENV, "eia")?;
        let endpoint = format!(
            "{}/{}/data/",
            self.base_url().trim_end_matches('/'),
            query.report.route(),
        );
        let params = [
            ("api_key", api_key),
            ("frequency", query.report.frequency().to_string()),
            ("data[0]", "value".to_string()),
            ("sort[0][column]", "period".to_string()),
            ("sort[0][direction]", "desc".to_string()),
            ("length", query.length.to_string()),
        ];
        let client = tdw_core::http_support::build_client(USER_AGENT, "eia build client")?;
        let response = client
            .get(&endpoint)
            .query(&params)
            .send()
            .await
            .map_err(|e| Error::Provider(format!("eia report request: {e}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            return Err(Error::Provider(format!(
                "eia report returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|e| Error::Provider(format!("eia report read body: {e}")))
    }

    fn transform_data(
        &self,
        query: &EiaReportQuery,
        raw: Bytes,
    ) -> Result<Vec<CommodityReportRow>> {
        let envelope: EiaReportEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("eia report parse_json: {e}")))?;
        let report = query.report.id();
        let mut rows = Vec::with_capacity(envelope.response.data.len());
        for row in envelope.response.data {
            if row.period.is_empty() {
                return Err(Error::Provider("eia report row missing period".to_string()));
            }
            // The value column is a string in EIA v2; skip missing markers and
            // parse the rest, matching the spot-price/natural-gas contract.
            let raw_value = row.value.trim();
            let value = if raw_value.is_empty() || raw_value == "." {
                None
            } else {
                Some(raw_value.parse::<f64>().map_err(|e| {
                    Error::Provider(format!(
                        "eia report value parse failed for {}: {e}",
                        row.period
                    ))
                })?)
            };
            let series_description = if row.series_description.is_empty() {
                None
            } else {
                Some(row.series_description)
            };
            let units = if row.units.is_empty() {
                None
            } else {
                Some(row.units)
            };
            rows.push(CommodityReportRow {
                report: report.to_string(),
                series_id: row.series,
                period: row.period,
                series_description,
                value,
                units,
            });
        }
        Ok(rows)
    }
}
