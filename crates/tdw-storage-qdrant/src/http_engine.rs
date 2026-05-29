//! Real Qdrant backend for `tdw_core::VectorEngine`.
//!
//! Gated by the `qdrant` feature. Talks to Qdrant over its REST API
//! (port 6333 by default), so no SDK crate is required — just
//! `reqwest`. Works with self-hosted Qdrant, Qdrant Cloud, or any
//! compatible service.
//!
//! Authentication is optional: if the constructor receives an
//! `api_key`, it is sent in the `api-key` header on every request.
//!
//! Point ID handling in this slice: Qdrant accepts point IDs as
//! unsigned integers or UUID strings. The engine forwards the
//! caller's `id` field verbatim; if it does not parse as either
//! shape on Qdrant's side, the upsert fails with the server's
//! error message. Normalising arbitrary strings to deterministic
//! UUIDs (e.g. via UUID v5) is a follow-up and is captured in the
//! readiness worksheet.

use std::collections::HashSet;
use std::sync::Mutex;

use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_core::{Error, Result, ScoredPoint, VectorEngine, VectorPoint, VectorQuery};

/// Production Qdrant backend.
pub struct QdrantHttpEngine {
    client: Client,
    base_url: Url,
    api_key: Option<String>,
    distance: String,
    ensured: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for QdrantHttpEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QdrantHttpEngine")
            .field("base_url", &self.base_url.as_str())
            .field("api_key", &self.api_key.as_ref().map(|_| "REDACTED"))
            .field("distance", &self.distance)
            .finish()
    }
}

impl QdrantHttpEngine {
    /// Build a Qdrant HTTP client. `endpoint` is the base URL
    /// (e.g. `http://127.0.0.1:6333`). `api_key` is optional;
    /// supply it for managed Qdrant deployments that require it.
    /// `distance` defaults to `Cosine`; pass `with_distance` to
    /// override (e.g. `"Euclid"` or `"Dot"`).
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(endpoint: &str, api_key: Option<String>) -> Result<Self> {
        let base_url = Url::parse(endpoint)
            .map_err(|error| Error::Storage(format!("qdrant endpoint: {error}")))?;
        let client = Client::builder()
            .user_agent("tdw-storage-qdrant/0.1")
            .build()
            .map_err(|error| Error::Storage(format!("qdrant client: {error}")))?;
        Ok(Self {
            client,
            base_url,
            api_key,
            distance: "Cosine".to_string(),
            ensured: Mutex::new(HashSet::new()),
        })
    }

    /// Override the vector distance metric used when this engine
    /// auto-creates a collection on first upsert. Valid Qdrant values
    /// are `"Cosine"`, `"Dot"`, and `"Euclid"`.
    #[must_use]
    pub fn with_distance(mut self, distance: impl Into<String>) -> Self {
        self.distance = distance.into();
        self
    }

    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let url = self
            .base_url
            .join(path)
            .map_err(|error| Error::Storage(format!("qdrant url: {error}")))?;
        let mut builder = self.client.request(method, url);
        if let Some(key) = self.api_key.as_deref() {
            builder = builder.header("api-key", key);
        }
        Ok(builder)
    }

    /// Idempotently create the collection if this engine instance has
    /// not seen it before. Inspects the existence endpoint first to
    /// avoid noisy 409s in the server log.
    ///
    /// Public so deployment bootstrap (`tdw-bootstrap`) can pre-create a
    /// baseline collection instead of relying on lazy creation at first
    /// upsert.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn ensure_collection(&self, name: &str, vector_size: usize) -> Result<()> {
        if let Ok(seen) = self.ensured.lock()
            && seen.contains(name)
        {
            return Ok(());
        }

        let exists_path = format!("/collections/{name}/exists");
        let exists_response = self
            .request(reqwest::Method::GET, &exists_path)?
            .send()
            .await
            .map_err(|error| Error::Storage(format!("qdrant exists: {error}")))?;
        if exists_response.status().is_success() {
            let body: Value = exists_response
                .json()
                .await
                .map_err(|error| Error::Storage(format!("qdrant exists body: {error}")))?;
            if body
                .get("result")
                .and_then(|result| result.get("exists"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                if let Ok(mut seen) = self.ensured.lock() {
                    seen.insert(name.to_string());
                }
                return Ok(());
            }
        }

        let create_path = format!("/collections/{name}");
        let create_body = json!({
            "vectors": {
                "size": vector_size,
                "distance": self.distance,
            }
        });
        let create_response = self
            .request(reqwest::Method::PUT, &create_path)?
            .json(&create_body)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("qdrant create_collection: {error}")))?;
        if !create_response.status().is_success() {
            let status = create_response.status();
            let body = create_response.text().await.unwrap_or_default();
            return Err(Error::Storage(format!(
                "qdrant create_collection returned {status}: {body}"
            )));
        }
        if let Ok(mut seen) = self.ensured.lock() {
            seen.insert(name.to_string());
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct SearchResultEnvelope {
    result: Vec<SearchHit>,
}

#[derive(Deserialize)]
struct SearchHit {
    id: Value,
    score: f32,
    payload: Option<Value>,
}

fn hit_id_to_string(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

#[async_trait]
impl VectorEngine for QdrantHttpEngine {
    async fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<()> {
        if points.is_empty() {
            return Err(Error::Storage(
                "qdrant upsert must include at least one point".to_string(),
            ));
        }
        let expected_dimension = points[0].vector.len();
        for point in &points {
            if point.vector.len() != expected_dimension {
                return Err(Error::Storage(format!(
                    "qdrant vector dimension mismatch for point {}: expected={}, actual={}",
                    point.id,
                    expected_dimension,
                    point.vector.len()
                )));
            }
        }
        self.ensure_collection(collection, expected_dimension)
            .await?;

        let body = json!({
            "points": points
                .iter()
                .map(|point| {
                    json!({
                        "id": point.id,
                        "vector": point.vector,
                        "payload": point.payload,
                    })
                })
                .collect::<Vec<_>>()
        });
        let path = format!("/collections/{collection}/points?wait=true");
        let response = self
            .request(reqwest::Method::PUT, &path)?
            .json(&body)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("qdrant upsert: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            return Err(Error::Storage(format!(
                "qdrant upsert returned {status}: {detail}"
            )));
        }
        Ok(())
    }

    async fn search_knn(&self, collection: &str, query: VectorQuery) -> Result<Vec<ScoredPoint>> {
        if query.vector.is_empty() {
            return Err(Error::Storage(
                "qdrant search vector must not be empty".to_string(),
            ));
        }
        let path = format!("/collections/{collection}/points/search");
        let body = json!({
            "vector": query.vector,
            "limit": query.top_k,
            "with_payload": true,
        });
        let response = self
            .request(reqwest::Method::POST, &path)?
            .json(&body)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("qdrant search: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            return Err(Error::Storage(format!(
                "qdrant search returned {status}: {detail}"
            )));
        }
        let envelope: SearchResultEnvelope = response
            .json()
            .await
            .map_err(|error| Error::Storage(format!("qdrant search body: {error}")))?;
        Ok(envelope
            .result
            .into_iter()
            .map(|hit| ScoredPoint {
                id: hit_id_to_string(&hit.id),
                score: hit.score,
                payload: hit.payload.unwrap_or(Value::Null),
            })
            .collect())
    }
}
