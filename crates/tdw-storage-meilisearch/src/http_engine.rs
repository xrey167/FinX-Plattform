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
            .finish()
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
                            .map(|error| error.message)
                            .unwrap_or_else(|| "no error message".to_string())
                    )));
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(TASK_POLL_INTERVAL_MS)).await;
                }
            }
        }
        Err(Error::Storage(format!(
            "meilisearch task {task_uid} did not reach terminal state within \
             {TASK_POLL_MAX_ATTEMPTS} attempts ({}ms each)",
            TASK_POLL_INTERVAL_MS
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
                    .map(|value| value as f32)
                    .unwrap_or(1.0);
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
