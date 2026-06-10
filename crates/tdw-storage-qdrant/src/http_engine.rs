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
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_core::{Error, Result, ScoredPoint, VectorEngine, VectorPoint, VectorQuery};

/// Cap on connection establishment so a stalled/black-holed Qdrant endpoint
/// fails fast instead of hanging the calling op (QD2/IO1).
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-request timeout — a backstop against a wedged-but-connected socket on
/// any single ensure/upsert/search request (QD2/IO1).
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

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
            .finish_non_exhaustive()
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
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .timeout(DEFAULT_REQUEST_TIMEOUT)
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
                // The collection already exists — verify its vector size matches
                // what we are about to write. A mismatch (e.g. an 8-dim hash
                // collection reused for a 1536-dim model) would otherwise corrupt
                // upserts/searches silently, so fail loudly instead.
                self.verify_vector_size(name, vector_size).await?;
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
            // The exists-check and this create PUT are separate awaits, and the
            // engine is shared across daemon connections, so two concurrent
            // first-upserts can both see exists=false and both issue the create.
            // The race-loser gets 409 Conflict ("collection already exists").
            // Qdrant collection creation is effectively idempotent, so a 409 is
            // not a real failure — treat it as success (still verifying the
            // vector size, exactly as the exists branch does, so a colliding
            // collection with an incompatible dimension is caught).
            if create_conflict_is_benign(status) {
                self.verify_vector_size(name, vector_size).await?;
                if let Ok(mut seen) = self.ensured.lock() {
                    seen.insert(name.to_string());
                }
                return Ok(());
            }
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

    /// Fetch an existing collection's config and error if its (single, unnamed)
    /// vector size differs from `expected`. A config we cannot read or parse is
    /// treated as "cannot verify" and allowed (best effort) rather than blocking
    /// an otherwise valid write.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Storage`] if the collection's vector size is known and
    /// does not equal `expected`.
    async fn verify_vector_size(&self, name: &str, expected: usize) -> Result<()> {
        let path = format!("/collections/{name}");
        let response = self
            .request(reqwest::Method::GET, &path)?
            .send()
            .await
            .map_err(|error| Error::Storage(format!("qdrant get_collection: {error}")))?;
        if !response.status().is_success() {
            return Ok(());
        }
        let body: Value = response
            .json()
            .await
            .map_err(|error| Error::Storage(format!("qdrant get_collection body: {error}")))?;
        if let Some(actual) = existing_vector_size(&body)
            && actual != expected as u64
        {
            return Err(Error::Storage(format!(
                "qdrant collection {name} has vector size {actual} but {expected} was requested; \
                 refusing a dimension-mismatched collection (use a model-specific collection)"
            )));
        }
        Ok(())
    }
}

/// Extract a single unnamed vector's `size` from a Qdrant `GET /collections/{name}`
/// body (`result.config.params.vectors.size`). Returns `None` for a named-vector
/// config or an unexpected shape (treated as "cannot verify").
fn existing_vector_size(body: &Value) -> Option<u64> {
    body.pointer("/result/config/params/vectors/size")
        .and_then(Value::as_u64)
}

/// Whether a non-success status from the create-collection `PUT` is a benign
/// "collection already exists" race rather than a genuine failure.
///
/// Only `409 Conflict` qualifies — the status Qdrant returns when a concurrent
/// first-upsert created the collection between our exists-check and this PUT.
/// Every other non-success status (4xx auth/validation, 5xx server) remains a
/// real error, so this predicate must stay strictly `== CONFLICT`.
fn create_conflict_is_benign(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::CONFLICT
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
        let mut body = json!({
            "vector": query.vector,
            "limit": query.top_k,
            "with_payload": true,
        });
        // Pre-B1 wire shape is preserved for unfiltered queries: the `filter`
        // key is only present when conditions exist.
        if !query.filter.is_empty()
            && let Value::Object(map) = &mut body
        {
            map.insert("filter".to_string(), qdrant_filter(&query.filter));
        }
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

/// Map the shared [`PayloadFilter`] onto Qdrant's filter DSL.
///
/// `MatchString`/`MatchAny` map to `match.value`/`match.any` (which on
/// array-valued payload fields mean "contains" — the same semantics the
/// in-memory engine's shared evaluator implements). `RangeString` maps to a
/// `range` condition, which Qdrant evaluates as a datetime range when the
/// payload field holds RFC 3339 strings — the only form the tdw-core contract
/// permits for range keys.
fn qdrant_filter(filter: &tdw_core::PayloadFilter) -> Value {
    let must: Vec<Value> = filter
        .must
        .iter()
        .map(|condition| match condition {
            tdw_core::PayloadCondition::MatchString { key, value } => json!({
                "key": key, "match": { "value": value },
            }),
            tdw_core::PayloadCondition::MatchAny { key, values } => json!({
                "key": key, "match": { "any": values },
            }),
            tdw_core::PayloadCondition::RangeString { key, gte, lte } => {
                let mut range = serde_json::Map::new();
                if let Some(bound) = gte {
                    range.insert("gte".to_string(), Value::String(bound.clone()));
                }
                if let Some(bound) = lte {
                    range.insert("lte".to_string(), Value::String(bound.clone()));
                }
                json!({ "key": key, "range": Value::Object(range) })
            }
        })
        .collect();
    json!({ "must": must })
}

#[cfg(test)]
mod tests {
    use super::{create_conflict_is_benign, existing_vector_size, qdrant_filter};
    use serde_json::json;

    #[test]
    fn qdrant_filter_maps_every_condition_form() {
        use tdw_core::{PayloadCondition, PayloadFilter};
        let mapped = qdrant_filter(&PayloadFilter {
            must: vec![
                PayloadCondition::MatchString {
                    key: "plane".to_string(),
                    value: "platform".to_string(),
                },
                PayloadCondition::MatchAny {
                    key: "tags".to_string(),
                    values: vec!["asset:equity".to_string(), "asset:crypto".to_string()],
                },
                PayloadCondition::RangeString {
                    key: "as_of".to_string(),
                    gte: None,
                    lte: Some("2026-01-01T00:00:00Z".to_string()),
                },
            ],
        });
        assert_eq!(
            mapped,
            json!({ "must": [
                { "key": "plane", "match": { "value": "platform" } },
                { "key": "tags", "match": { "any": ["asset:equity", "asset:crypto"] } },
                { "key": "as_of", "range": { "lte": "2026-01-01T00:00:00Z" } },
            ]})
        );
    }

    #[test]
    fn only_409_conflict_is_treated_as_benign_already_exists() {
        use reqwest::StatusCode;
        // A creation race-loser gets 409 and must be treated as success.
        assert!(create_conflict_is_benign(StatusCode::CONFLICT));
        // Every other non-success status is a genuine failure and must still
        // surface as an error (guards against broadening this to !is_success()).
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert!(!create_conflict_is_benign(status), "{status} must error");
        }
    }

    #[test]
    fn existing_vector_size_reads_single_vector_config() {
        let body = json!({
            "result": { "config": { "params": { "vectors": { "size": 1536, "distance": "Cosine" } } } }
        });
        assert_eq!(existing_vector_size(&body), Some(1536));
    }

    #[test]
    fn existing_vector_size_is_none_for_unexpected_shape() {
        // Named-vector configs / odd shapes are "cannot verify" (None), so the
        // guard allows the write rather than blocking on a config it can't read.
        assert_eq!(existing_vector_size(&json!({ "result": {} })), None);
        assert_eq!(existing_vector_size(&json!({})), None);
    }
}
