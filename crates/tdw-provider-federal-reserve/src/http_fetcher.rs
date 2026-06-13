//! Real Federal Reserve HTTP fetchers for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks directly to public Fed data portals
//! (`https://www.federalreserve.gov`) without authentication. Live calls are
//! gated by `TDW_FEDERAL_RESERVE_LIVE=1` in the integration tests.
//!
//! Scope (keyless government wave, pragmatic): the macro-series fetcher
//! standardizes the H.6 money-measures table and the primary-dealer net
//! positioning statistic into [`tdw_domain::MacroSeries`] from a Fed
//! observation-list JSON shape; the FOMC fetcher standardizes the FOMC document
//! listing into [`tdw_domain::FomcDocument`]. The exact Fed Data Download
//! Program (DDP) request URLs vary per release; the offline `transform_data`
//! parse path over a recorded observation/document list is the standardized
//! contract, and a live attempt uses the documented release endpoints.

#![cfg(feature = "http")]

use serde::Deserialize;
use tdw_core::http_support::prelude::*;
use tdw_core::query_params::StandardParams;
use tdw_domain::{FomcDocument, MacroSeries};

use crate::{BASE_URL, FedCatalogQuery, FedModel};

const USER_AGENT: &str = "tdw-provider-federal-reserve/0.1 (contact@finx.example)";

/// Map the resolved command to the Fed release resource path. The H.6 and
/// primary-dealer releases expose a downloadable data resource; the exact path
/// is release-specific, so this keeps the relative resource the fetcher GETs.
fn release_path(command: &str) -> &'static str {
    match command {
        "fixedincome/government/dealer_stats" | "economy/primary_dealer_positioning" => {
            "/releases/pd/dataviz/pd/series.json"
        }
        "economy/primary_dealer_fails" => "/releases/pd/dataviz/pd/fails.json",
        "economy/central_bank_holdings" => "/releases/h41/current/h41.json",
        // economy/money_measures (H.6) is the default macro release.
        _ => "/releases/h6/current/h6.json",
    }
}

/// Perform a Fed release GET, returning the raw response bytes.
async fn fetch_release(base_url: &str, path: &str, ctx: &str) -> Result<Bytes> {
    let endpoint = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let client =
        tdw_core::http_support::build_client(USER_AGENT, "federal_reserve http client build")?;
    let response = client
        .get(&endpoint)
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

/// A Fed observation list `{ "observations": [ { "date", "value" } ] }`. The
/// `value` is decoded as a string (Fed releases report numeric values as
/// strings, with a `"."`/empty missing-value sentinel like FRED).
#[derive(Deserialize)]
struct FedObservationEnvelope {
    #[serde(default)]
    observations: Vec<FedObservation>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct FedObservation {
    #[serde(default)]
    date: String,
    #[serde(default)]
    value: String,
}

/// Apply the shared `limit` to a decoded row vec (the Fed releases return the
/// full series; date filtering is applied client-side in `transform_data`).
fn truncate_to_limit<T>(rows: Vec<T>, params: &StandardParams) -> Vec<T> {
    match params.limit {
        Some(limit) if limit > 0 => rows.into_iter().take(limit as usize).collect(),
        _ => rows,
    }
}

// ── Macro-series fetcher (economy/money_measures, dealer_stats) ───────────────

tdw_core::provider_fetcher_struct!(
    /// Production Federal Reserve macro-series fetcher.
    ///
    /// Standardizes `economy/money_measures` (H.6) and
    /// `fixedincome/government/dealer_stats` (primary-dealer net positioning) to
    /// [`tdw_domain::MacroSeries`] rows.
    pub FedMacroSeriesHttpFetcher,
    BASE_URL
);

#[async_trait]
impl Fetcher<FedCatalogQuery, MacroSeries> for FedMacroSeriesHttpFetcher {
    const PROVIDER: &'static str = "federal_reserve";
    const ENDPOINT: &'static str = "macro_series";

    fn transform_query(params: Value) -> Result<FedCatalogQuery> {
        let query = FedCatalogQuery::from_value(&params)?;
        if query.endpoint().model != FedModel::Macro {
            return Err(Error::InvalidQuery(format!(
                "federal_reserve macro fetcher does not serve command {}",
                query.command
            )));
        }
        Ok(query)
    }

    async fn extract_data(&self, query: &FedCatalogQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_release(
            self.base_url(),
            release_path(&query.command),
            "federal_reserve macro_series",
        )
        .await
    }

    fn transform_data(&self, query: &FedCatalogQuery, raw: Bytes) -> Result<Vec<MacroSeries>> {
        let endpoint = query.endpoint();
        let envelope: FedObservationEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("federal_reserve macro parse_json: {e}")))?;
        if let Some(error) = envelope.error {
            return Err(Error::Provider(format!(
                "federal_reserve macro api error: {error}"
            )));
        }
        let mut rows = Vec::with_capacity(envelope.observations.len());
        for observation in envelope.observations {
            if observation.date.is_empty() {
                continue;
            }
            let raw_value = observation.value.trim();
            let value = if raw_value.is_empty() || raw_value == "." {
                None
            } else {
                Some(raw_value.replace(',', "").parse::<f64>().map_err(|e| {
                    Error::Provider(format!(
                        "federal_reserve value parse failed for {}: {e}",
                        observation.date
                    ))
                })?)
            };
            rows.push(MacroSeries {
                series_id: endpoint.series_id.to_string(),
                title: Some(endpoint.title.to_string()),
                date: observation.date,
                value,
                country: None,
                frequency: Some(endpoint.frequency.to_string()),
                unit: Some(endpoint.unit.to_string()),
                transform: None,
            });
        }
        Ok(truncate_to_limit(rows, &query.params))
    }
}

// ── FOMC documents fetcher (regulators/fed/fomc_documents) ────────────────────

tdw_core::provider_fetcher_struct!(
    /// Production Federal Reserve FOMC document-index fetcher.
    ///
    /// Standardizes `regulators/fed/fomc_documents` to
    /// [`tdw_domain::FomcDocument`] rows from the FOMC document listing.
    pub FedFomcDocumentsHttpFetcher,
    BASE_URL
);

/// The FOMC document listing `{ "documents": [ { "type", "date", "title",
/// "url" } ] }`.
#[derive(Deserialize)]
struct FomcDocumentEnvelope {
    #[serde(default)]
    documents: Vec<FomcDocumentRow>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct FomcDocumentRow {
    #[serde(default, rename = "type")]
    doc_type: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[async_trait]
impl Fetcher<FedCatalogQuery, FomcDocument> for FedFomcDocumentsHttpFetcher {
    const PROVIDER: &'static str = "federal_reserve";
    const ENDPOINT: &'static str = "fomc_documents";

    fn transform_query(params: Value) -> Result<FedCatalogQuery> {
        let query = FedCatalogQuery::from_value(&params)?;
        if query.endpoint().model != FedModel::FomcDocuments {
            return Err(Error::InvalidQuery(format!(
                "federal_reserve fomc fetcher does not serve command {}",
                query.command
            )));
        }
        Ok(query)
    }

    async fn extract_data(&self, _query: &FedCatalogQuery, _creds: &Credentials) -> Result<Bytes> {
        fetch_release(
            self.base_url(),
            "/json/fomc_historical.json",
            "federal_reserve fomc_documents",
        )
        .await
    }

    fn transform_data(&self, query: &FedCatalogQuery, raw: Bytes) -> Result<Vec<FomcDocument>> {
        let envelope: FomcDocumentEnvelope = serde_json::from_slice(&raw)
            .map_err(|e| Error::Provider(format!("federal_reserve fomc parse_json: {e}")))?;
        if let Some(error) = envelope.error {
            return Err(Error::Provider(format!(
                "federal_reserve fomc api error: {error}"
            )));
        }
        let mut rows = Vec::with_capacity(envelope.documents.len());
        for document in envelope.documents {
            let Some(doc_type) = document.doc_type.filter(|t| !t.trim().is_empty()) else {
                continue;
            };
            rows.push(FomcDocument {
                doc_type,
                date: document.date,
                title: document.title,
                url: document.url,
            });
        }
        Ok(truncate_to_limit(rows, &query.params))
    }
}
