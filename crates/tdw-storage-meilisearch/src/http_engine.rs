//! Real Meilisearch backend for `tdw_core::LexicalEngine`.
//!
//! Gated by the `meilisearch` feature. Talks to Meilisearch over its
//! REST API (port 7700 by default), so no SDK crate is required —
//! just `reqwest`. Works with self-hosted Meilisearch or Meilisearch
//! Cloud.
//!
//! Authentication is optional: if the constructor receives an
//! `api_key`, it is sent in the `Authorization: Bearer ...` header on
//! every request. Self-hosted dev instances with `MEILI_NO_ANALYTICS`
//! typically don't require a key.
//!
//! Index documents are async on Meilisearch's side: `POST /documents`
//! returns a task UID, and the document is only searchable after the
//! task reaches `succeeded`. The engine polls `/tasks/{uid}` (up to
//! ~10s by default) before returning from `index`, so callers can
//! immediately follow an `index` with `search_text` without flakiness.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_core::{Error, LexicalDoc, LexicalEngine, Result, ScoredDoc, TextQuery};

const TASK_POLL_INTERVAL_MS: u64 = 200;
const TASK_POLL_MAX_ATTEMPTS: u32 = 60;

/// Production Meilisearch backend.
#[derive(Clone)]
pub struct MeilisearchHttpEngine {
    client: Client,
    base_url: Url,
    api_key: Option<String>,
}

impl std::fmt::Debug for MeilisearchHttpEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MeilisearchHttpEngine")
            .field("base_url", &self.base_url.as_str())
            .field("api_key", &self.api_key.as_ref().map(|_| "REDACTED"))
            .finish_non_exhaustive()
    }
}

impl MeilisearchHttpEngine {
    /// Build a Meilisearch HTTP client. `endpoint` is the base URL
    /// (e.g. `http://127.0.0.1:7700`). `api_key` is optional; supply
    /// it for managed deployments or self-hosted instances with a
    /// configured master key.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(endpoint: &str, api_key: Option<String>) -> Result<Self> {
        let base_url = Url::parse(endpoint)
            .map_err(|error| Error::Storage(format!("meilisearch endpoint: {error}")))?;
        let client = Client::builder()
            .user_agent("tdw-storage-meilisearch/0.1")
            .build()
            .map_err(|error| Error::Storage(format!("meilisearch client: {error}")))?;
        Ok(Self {
            client,
            base_url,
            api_key,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let url = self
            .base_url
            .join(path)
            .map_err(|error| Error::Storage(format!("meilisearch url: {error}")))?;
        let mut builder = self.client.request(method, url);
        if let Some(key) = self.api_key.as_deref() {
            builder = builder.bearer_auth(key);
        }
        Ok(builder)
    }

    /// Block until the given Meilisearch task reaches a terminal
    /// state (succeeded/failed/canceled). Returns the task status on
    /// success; surfaces the Meilisearch error envelope on failure.
    async fn wait_for_task(&self, task_uid: u64) -> Result<()> {
        let path = format!("/tasks/{task_uid}");
        for _ in 0..TASK_POLL_MAX_ATTEMPTS {
            let response = self
                .request(reqwest::Method::GET, &path)?
                .send()
                .await
                .map_err(|error| Error::Storage(format!("meilisearch task: {error}")))?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(Error::Storage(format!(
                    "meilisearch task {task_uid} returned {status}: {body}"
                )));
            }
            let task: TaskEnvelope = response
                .json()
                .await
                .map_err(|error| Error::Storage(format!("meilisearch task body: {error}")))?;
            match task.status.as_str() {
                "succeeded" => return Ok(()),
                "failed" | "canceled" => {
                    return Err(Error::Storage(format!(
                        "meilisearch task {task_uid} {}: {}",
                        task.status,
                        task.error
                            .map_or_else(|| "no error message".to_string(), |error| error.message)
                    )));
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(TASK_POLL_INTERVAL_MS)).await;
                }
            }
        }
        Err(Error::Storage(format!(
            "meilisearch task {task_uid} did not reach terminal state within \
             {TASK_POLL_MAX_ATTEMPTS} attempts ({TASK_POLL_INTERVAL_MS}ms each)"
        )))
    }

    /// Idempotently create the index with the given primary key if it does
    /// not already exist.
    ///
    /// Public so deployment bootstrap (`tdw-bootstrap`) can pre-create a
    /// baseline index instead of relying on lazy creation at first write.
    ///
    /// # Errors
    ///
    /// Returns an error if the existence check, the create request, or the
    /// resulting Meilisearch task fails.
    pub async fn ensure_index(&self, index: &str, primary_key: &str) -> Result<()> {
        validate_index(index)?;
        let exists = self
            .request(reqwest::Method::GET, &format!("/indexes/{index}"))?
            .send()
            .await
            .map_err(|error| Error::Storage(format!("meilisearch index exists: {error}")))?;
        if exists.status().is_success() {
            return Ok(());
        }

        let body = json!({ "uid": index, "primaryKey": primary_key });
        let response = self
            .request(reqwest::Method::POST, "/indexes")?
            .json(&body)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("meilisearch create index: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(Error::Storage(format!(
                "meilisearch create index returned {status}: {text}"
            )));
        }
        let enqueued: EnqueueResponse = response
            .json()
            .await
            .map_err(|error| Error::Storage(format!("meilisearch create index body: {error}")))?;
        self.wait_for_task(enqueued.task_uid).await
    }
}

#[derive(Deserialize)]
struct TaskEnvelope {
    status: String,
    #[serde(default)]
    error: Option<TaskError>,
}

#[derive(Deserialize)]
struct TaskError {
    message: String,
}

#[derive(Deserialize)]
struct EnqueueResponse {
    #[serde(rename = "taskUid")]
    task_uid: u64,
}

#[derive(Deserialize)]
struct SearchEnvelope {
    hits: Vec<Value>,
}

/// Validate a Meilisearch index UID before it is interpolated into a request
/// path such as `/indexes/{index}/documents`.
///
/// Meilisearch index UIDs are limited to ASCII alphanumerics, `-` and `_`.
/// Enforcing that here means an `index` containing `/`, `?`, `#` or `..` can no
/// longer alter the request path or smuggle query parameters via
/// [`Url::join`], which treats those characters structurally.
fn validate_index(index: &str) -> Result<()> {
    if index.is_empty()
        || !index
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(Error::Storage(format!(
            "meilisearch index uid {index:?} must be non-empty and contain only \
             alphanumeric characters, '-' or '_'"
        )));
    }
    Ok(())
}

fn flatten_doc(doc: LexicalDoc) -> Value {
    let mut document = json!({
        "id": doc.id,
        "body": doc.body,
    });
    if let Value::Object(extra) = doc.fields
        && let Value::Object(target) = &mut document
    {
        for (key, value) in extra {
            target.entry(key).or_insert(value);
        }
    }
    document
}

fn hit_id_to_string(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

#[async_trait]
impl LexicalEngine for MeilisearchHttpEngine {
    async fn index(&self, index: &str, docs: Vec<LexicalDoc>) -> Result<()> {
        validate_index(index)?;
        if docs.is_empty() {
            return Err(Error::Storage(
                "meilisearch index must include at least one document".to_string(),
            ));
        }
        let documents: Vec<Value> = docs.into_iter().map(flatten_doc).collect();
        let path = format!("/indexes/{index}/documents?primaryKey=id");
        let response = self
            .request(reqwest::Method::POST, &path)?
            .json(&documents)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("meilisearch index: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Storage(format!(
                "meilisearch index returned {status}: {body}"
            )));
        }
        let enqueued: EnqueueResponse = response
            .json()
            .await
            .map_err(|error| Error::Storage(format!("meilisearch index body: {error}")))?;
        self.wait_for_task(enqueued.task_uid).await
    }

    async fn search_text(&self, index: &str, query: TextQuery) -> Result<Vec<ScoredDoc>> {
        validate_index(index)?;
        if query.text.trim().is_empty() {
            return Err(Error::Storage(
                "meilisearch search text must not be empty".to_string(),
            ));
        }
        let path = format!("/indexes/{index}/search");
        let body = json!({
            "q": query.text,
            "limit": query.top_k,
            "showRankingScore": true,
        });
        let response = self
            .request(reqwest::Method::POST, &path)?
            .json(&body)
            .send()
            .await
            .map_err(|error| Error::Storage(format!("meilisearch search: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Storage(format!(
                "meilisearch search returned {status}: {body}"
            )));
        }
        let envelope: SearchEnvelope = response
            .json()
            .await
            .map_err(|error| Error::Storage(format!("meilisearch search body: {error}")))?;
        Ok(envelope
            .hits
            .into_iter()
            .map(|mut hit| {
                let id = hit.get("id").map(hit_id_to_string).unwrap_or_default();
                // Meilisearch ranking scores are in [0.0, 1.0]; narrowing the
                // f64 to the ScoredDoc f32 score loses no meaningful precision.
                #[allow(clippy::cast_possible_truncation)]
                let score = hit
                    .get("_rankingScore")
                    .and_then(Value::as_f64)
                    .map_or(1.0, |value| value as f32);
                // Strip Meilisearch's reserved fields before returning
                // the document to the caller.
                if let Value::Object(map) = &mut hit {
                    map.remove("_rankingScore");
                }
                ScoredDoc {
                    id,
                    score,
                    fields: hit,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_index_accepts_uid_grammar_and_rejects_injection() {
        // Legitimate Meilisearch UIDs.
        for ok in ["docs", "lexical-1", "tdw_index", "ABC123"] {
            assert!(validate_index(ok).is_ok(), "{ok:?} should be accepted");
        }
        // Empty and path/query-injection payloads.
        for bad in ["", "a/b", "a?x=1", "..", "idx#frag", "with space"] {
            assert!(validate_index(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[tokio::test]
    async fn index_rejects_injection_index_before_any_request() {
        // `new` only parses the URL; no server is contacted. Because
        // validate_index runs before the empty-docs check (and before the
        // network call), a crafted index fails fast with a validation error
        // rather than a connection error.
        let engine = MeilisearchHttpEngine::new("http://127.0.0.1:7700", None)
            .unwrap_or_else(|error| panic!("engine builds: {error}"));
        let error = engine
            .index("evil/../../x", Vec::new())
            .await
            .expect_err("injection index must be rejected");
        match error {
            Error::Storage(message) => assert!(
                message.contains("index uid"),
                "expected index-uid validation error, got: {message}"
            ),
            other => panic!("expected Error::Storage, got: {other:?}"),
        }
    }
}
