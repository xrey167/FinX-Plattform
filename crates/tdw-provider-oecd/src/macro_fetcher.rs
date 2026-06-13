//! Real OECD SDMX-JSON macro-series fetcher for `tdw_core::Fetcher`
//! (`OpenBB`-parity item **P4W4**).
//!
//! Gated by the `http` feature. Resolves an `economy/*` catalog `command` (via
//! [`crate::catalog`]) to an OECD SDMX dataset + default dimension filter, fetches
//! the SDMX-JSON `data` slice from the public OECD Statistics API
//! (`https://stats.oecd.org/SDMX-JSON/data`), and normalizes each observation to a
//! standardized [`tdw_domain::MacroSeries`] row — the raw SDMX shapes never leak
//! past this module. No API key is required; the live integration test is
//! additionally gated by `TDW_OECD_LIVE=1` so unattended CI stays offline.

#![cfg(feature = "http")]

use serde::Deserialize;
use serde_json::Value as JsonValue;
use tdw_core::http_support::prelude::*;
use tdw_domain::MacroSeries;

use crate::{BASE_URL, OecdCatalogQuery};

const USER_AGENT: &str = "tdw-provider-oecd/0.1 (contact@finx.example)";

// ── SDMX-JSON deserialisation types ───────────────────────────────────────────

/// Top-level OECD SDMX-JSON envelope (the `dataSets` + `structure` shape).
#[derive(Deserialize)]
struct SdmxEnvelope {
    #[serde(rename = "dataSets", default)]
    data_sets: Vec<SdmxDataSet>,
    #[serde(default)]
    structure: Option<SdmxStructure>,
}

#[derive(Deserialize)]
struct SdmxDataSet {
    #[serde(default)]
    observations: serde_json::Map<String, JsonValue>,
}

#[derive(Deserialize)]
struct SdmxStructure {
    #[serde(default)]
    dimensions: Option<SdmxDimensions>,
}

#[derive(Deserialize)]
struct SdmxDimensions {
    #[serde(default)]
    observation: Vec<SdmxDimension>,
}

#[derive(Deserialize)]
struct SdmxDimension {
    #[serde(default)]
    id: String,
    #[serde(default)]
    values: Vec<SdmxDimensionValue>,
}

#[derive(Deserialize)]
struct SdmxDimensionValue {
    #[serde(default)]
    id: String,
}

/// One decoded `(period, value)` observation from the SDMX `data` slice.
struct DecodedObservation {
    period: String,
    value: Option<f64>,
}

/// Decode an OECD SDMX-JSON `data` response into a flat list of dated
/// observations, mapping each observation index back to its `TIME_PERIOD` label.
///
/// The OECD error/empty response decodes to an empty (not malformed) result so an
/// out-of-range window does not error; a non-object envelope still errors.
fn decode_observations(raw: &Bytes) -> Result<Vec<DecodedObservation>> {
    let envelope: SdmxEnvelope = serde_json::from_slice(raw)
        .map_err(|error| Error::Provider(format!("oecd parse_json: {error}")))?;

    let time_values: Vec<String> = envelope
        .structure
        .as_ref()
        .and_then(|structure| structure.dimensions.as_ref())
        .map(|dimensions| {
            dimensions
                .observation
                .iter()
                .find(|dimension| dimension.id == "TIME_PERIOD")
                .map(|dimension| {
                    dimension
                        .values
                        .iter()
                        .map(|value| value.id.clone())
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let Some(data_set) = envelope.data_sets.into_iter().next() else {
        return Ok(Vec::new());
    };

    let mut rows = Vec::with_capacity(data_set.observations.len());
    for (obs_key, obs_val) in &data_set.observations {
        // The TIME_PERIOD index is the last colon-separated component of the key.
        let period = obs_key
            .split(':')
            .next_back()
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| time_values.get(index))
            .cloned()
            .unwrap_or_else(|| obs_key.clone());
        // Each observation is an array; index 0 is the numeric value (or null).
        let value = obs_val.get(0).and_then(JsonValue::as_f64);
        rows.push(DecodedObservation { period, value });
    }

    // Stable output order: sort by period label so emitted rows are deterministic.
    rows.sort_by(|a, b| a.period.cmp(&b.period));
    Ok(rows)
}

/// Perform an OECD SDMX-JSON `data` GET for the resolved dataset + filter,
/// returning the raw response bytes.
async fn fetch_sdmx_data(
    base_url: &str,
    dataset: &str,
    filter: &str,
    query: &OecdCatalogQuery,
) -> Result<Bytes> {
    let base = base_url.trim_end_matches('/');
    let mut url = format!("{base}/{dataset}/{filter}/OECD?contentType=application/json");
    if let Some(start) = query.params.start_date {
        url.push_str(&format!("&startTime={}", start.year()));
    }
    if let Some(end) = query.params.end_date {
        url.push_str(&format!("&endTime={}", end.year()));
    }
    let client = tdw_core::http_support::build_client(USER_AGENT, "oecd http client build")?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| Error::Provider(format!("oecd macro_series extract_data: {error}")))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(Error::Provider(format!(
            "oecd macro_series returned {status}: {body}"
        )));
    }
    response
        .bytes()
        .await
        .map_err(|error| Error::Provider(format!("oecd macro_series read body: {error}")))
}

tdw_core::provider_fetcher_struct!(
    /// Production OECD SDMX-JSON macro-series fetcher.
    ///
    /// Resolves an `economy/*` catalog `command` to an OECD dataset slice and
    /// normalizes the observations to [`tdw_domain::MacroSeries`] rows.
    pub OecdHttpMacroSeriesFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<OecdCatalogQuery, MacroSeries> for OecdHttpMacroSeriesFetcher {
    const PROVIDER: &'static str = "oecd";
    const ENDPOINT: &'static str = "macro_series";

    fn transform_query(params: Value) -> Result<OecdCatalogQuery> {
        OecdCatalogQuery::from_value(&params)
    }

    async fn extract_data(&self, query: &OecdCatalogQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_sdmx_data(self.base_url(), &query.dataset, &query.filter, query).await
    }

    fn transform_data(&self, query: &OecdCatalogQuery, raw: Bytes) -> Result<Vec<MacroSeries>> {
        let endpoint = query.endpoint();
        let limit = query
            .params
            .limit
            .map_or(usize::MAX, |limit| limit as usize);
        let observations = decode_observations(&raw)?;
        Ok(observations
            .into_iter()
            .take(limit)
            .map(|observation| MacroSeries {
                series_id: endpoint.dataset.to_string(),
                title: Some(endpoint.title.to_string()),
                date: observation.period,
                value: observation.value,
                country: None,
                frequency: Some(endpoint.frequency.to_string()),
                unit: Some(endpoint.unit.to_string()),
                transform: None,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Bytes {
        Bytes::from(
            serde_json::json!({
                "dataSets": [{
                    "observations": {
                        "0:0:0:0": [100.5, 0, null],
                        "0:0:0:1": [101.0, 0, null],
                        "0:0:0:2": [null]
                    }
                }],
                "structure": {
                    "dimensions": {
                        "observation": [{
                            "id": "TIME_PERIOD",
                            "values": [
                                { "id": "2023-Q1" },
                                { "id": "2023-Q2" },
                                { "id": "2023-Q3" }
                            ]
                        }]
                    }
                }
            })
            .to_string(),
        )
    }

    #[test]
    fn transform_data_normalizes_sdmx_to_macro_series() {
        let fetcher = OecdHttpMacroSeriesFetcher::default();
        let query = OecdCatalogQuery::new("economy/house_price_index")
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        let rows = fetcher
            .transform_data(&query, fixture())
            .unwrap_or_else(|e| panic!("transform should succeed: {e}"));

        assert_eq!(rows.len(), 3);
        // Deterministic period order.
        assert_eq!(rows[0].date, "2023-Q1");
        assert_eq!(rows[0].series_id, "HOUSE_PRICES");
        assert_eq!(rows[0].frequency.as_deref(), Some("quarterly"));
        assert_eq!(rows[0].unit.as_deref(), Some("index"));
        assert!((rows[1].value.expect("value") - 101.0).abs() < f64::EPSILON);
        // Null observation yields a row with no value (date never dropped).
        assert_eq!(rows[2].date, "2023-Q3");
        assert_eq!(rows[2].value, None);
    }

    #[test]
    fn transform_data_handles_empty_envelope() {
        let fetcher = OecdHttpMacroSeriesFetcher::default();
        let query = OecdCatalogQuery::new("economy/interest_rates")
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        let raw = Bytes::from(serde_json::json!({ "dataSets": [] }).to_string());
        let rows = fetcher
            .transform_data(&query, raw)
            .unwrap_or_else(|e| panic!("empty transform should succeed: {e}"));
        assert!(rows.is_empty());
    }

    #[test]
    fn transform_data_rejects_malformed_json() {
        let fetcher = OecdHttpMacroSeriesFetcher::default();
        let query = OecdCatalogQuery::new("economy/interest_rates")
            .unwrap_or_else(|e| panic!("query should build: {e}"));
        let raw = Bytes::from_static(b"not json");
        assert!(fetcher.transform_data(&query, raw).is_err());
    }

    #[test]
    fn transform_data_respects_limit() {
        let fetcher = OecdHttpMacroSeriesFetcher::default();
        let query = OecdCatalogQuery::from_value(&serde_json::json!({
            "command": "economy/house_price_index",
            "limit": 1
        }))
        .unwrap_or_else(|e| panic!("query should build: {e}"));
        let rows = fetcher
            .transform_data(&query, fixture())
            .unwrap_or_else(|e| panic!("transform should succeed: {e}"));
        assert_eq!(rows.len(), 1);
    }
}
