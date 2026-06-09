//! Real OpenAI Embeddings HTTP client for G012.
//!
//! Gated by the `http` feature. Posts to `POST /v1/embeddings`
//! against OpenAI or a compatible embedding gateway via
//! [`OpenAiEmbeddingHttpClient::with_base_url`]. The existing
//! request-builder helpers remain available for offline tests and
//! higher-level dispatchers.

use reqwest::{Client, Url};
use serde::Deserialize;
use tdw_embed::{Embedding, EmbeddingError, EmbeddingProvider};
use thiserror::Error;

use crate::{
    DEFAULT_BASE_URL, OpenAiEmbeddingAdapterError, build_embedding_request, decode_embedding,
};

/// Production OpenAI embeddings client.
#[derive(Clone)]
pub struct OpenAiEmbeddingHttpClient {
    client: Client,
    base_url: Url,
    api_key: String,
    model_id: String,
}

impl std::fmt::Debug for OpenAiEmbeddingHttpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiEmbeddingHttpClient")
            .field("base_url", &self.base_url.as_str())
            .field("model_id", &self.model_id)
            .field("api_key", &"REDACTED")
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum OpenAiEmbeddingHttpError {
    #[error("openai embedding api key must not be empty")]
    MissingApiKey,
    #[error("invalid base url: {0}")]
    InvalidBaseUrl(String),
    #[error("openai embedding adapter error: {0}")]
    Adapter(#[from] OpenAiEmbeddingAdapterError),
    #[error("openai embedding http {status}: {body}")]
    Http {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("response shape error: {0}")]
    InvalidResponse(String),
    #[error("client build error: {0}")]
    ClientBuild(String),
}

impl OpenAiEmbeddingHttpClient {
    /// Construct a client for an authenticated OpenAI embeddings
    /// endpoint. Defaults to `https://api.openai.com/v1`.
    pub fn new(
        api_key: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, OpenAiEmbeddingHttpError> {
        let api_key = normalize_api_key(api_key.into())?;
        let model_id = normalize_model(model_id.into())?;
        let base_url = parse_base_url(DEFAULT_BASE_URL)?;
        let client = Client::builder()
            .user_agent("tdw-embed-openai/0.1")
            // Bound the client so a stalled endpoint can't hang the knowledge-index
            // op forever (IO1): fail fast on connect, with a per-request backstop.
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|error| OpenAiEmbeddingHttpError::ClientBuild(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            api_key,
            model_id,
        })
    }

    /// Override the base URL. Supplying either an OpenAI-style `/v1`
    /// base or a full `/embeddings` endpoint is accepted.
    pub fn with_base_url(
        mut self,
        base_url: impl Into<String>,
    ) -> Result<Self, OpenAiEmbeddingHttpError> {
        self.base_url = parse_base_url(&base_url.into())?;
        Ok(self)
    }

    /// OpenAI embedding model id this client is configured for.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Post `input` to `POST /v1/embeddings` and decode the first
    /// returned vector into the workspace [`Embedding`] contract.
    pub async fn embed(&self, input: &str) -> Result<Embedding, OpenAiEmbeddingHttpError> {
        let request = build_embedding_request(&self.model_id, input, true)?;
        let response = self
            .client
            .post(embeddings_url(&self.base_url)?)
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .json(&request.body)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OpenAiEmbeddingHttpError::Http { status, body });
        }
        let envelope: EmbeddingsEnvelope = response.json().await?;
        parse_response(&self.model_id, envelope)
    }
}

/// Bridge the real OpenAI HTTP client onto the workspace
/// [`EmbeddingProvider`] trait so the knowledge index can drive it
/// behind `Arc<dyn EmbeddingProvider>`. The crate-specific
/// [`OpenAiEmbeddingHttpError`] is flattened onto
/// [`EmbeddingError::Provider`], preserving its message.
#[async_trait::async_trait]
impl EmbeddingProvider for OpenAiEmbeddingHttpClient {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn embed(&self, text: &str) -> tdw_embed::Result<Embedding> {
        OpenAiEmbeddingHttpClient::embed(self, text)
            .await
            .map_err(|error| EmbeddingError::Provider(error.to_string()))
    }
}

fn normalize_api_key(api_key: String) -> Result<String, OpenAiEmbeddingHttpError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(OpenAiEmbeddingHttpError::MissingApiKey);
    }
    Ok(api_key.to_string())
}

fn normalize_model(model_id: String) -> Result<String, OpenAiEmbeddingHttpError> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err(OpenAiEmbeddingAdapterError::EmptyModel.into());
    }
    Ok(model_id.to_string())
}

fn parse_base_url(base_url: &str) -> Result<Url, OpenAiEmbeddingHttpError> {
    if base_url
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(OpenAiEmbeddingHttpError::InvalidBaseUrl(
            "base url contains whitespace or control characters".to_string(),
        ));
    }
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err(OpenAiEmbeddingHttpError::InvalidBaseUrl(
            "base url must start with http:// or https://".to_string(),
        ));
    }
    Url::parse(base_url)
        .map_err(|error| OpenAiEmbeddingHttpError::InvalidBaseUrl(error.to_string()))
}

fn embeddings_url(base_url: &Url) -> Result<Url, OpenAiEmbeddingHttpError> {
    let mut url = base_url.clone();
    let path = url.path().trim_end_matches('/');
    let path = if path.ends_with("/embeddings") {
        path.to_string()
    } else if path.ends_with("/v1") {
        format!("{path}/embeddings")
    } else if path.is_empty() {
        "/v1/embeddings".to_string()
    } else {
        format!("{path}/embeddings")
    };
    url.set_path(&path);
    url.set_query(None);
    Ok(url)
}

#[derive(Deserialize)]
pub(crate) struct EmbeddingsEnvelope {
    #[serde(default)]
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
pub(crate) struct EmbeddingData {
    embedding: Vec<f32>,
}

/// Parse an OpenAI embeddings envelope into the workspace
/// [`Embedding`] contract. Extracted so cassette tests can exercise
/// decoder behavior without touching the network.
pub(crate) fn parse_response(
    model_id: &str,
    envelope: EmbeddingsEnvelope,
) -> Result<Embedding, OpenAiEmbeddingHttpError> {
    let vector = envelope
        .data
        .into_iter()
        .next()
        .map(|item| item.embedding)
        .ok_or_else(|| {
            OpenAiEmbeddingHttpError::InvalidResponse(
                "openai embedding response had no data entries".to_string(),
            )
        })?;
    decode_embedding(model_id, vector).map_err(OpenAiEmbeddingHttpError::Adapter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_constructor_rejects_empty_key_and_model_and_redacts_debug() {
        assert!(matches!(
            OpenAiEmbeddingHttpClient::new("", "text-embedding-3-small"),
            Err(OpenAiEmbeddingHttpError::MissingApiKey)
        ));
        assert!(matches!(
            OpenAiEmbeddingHttpClient::new("sk-test", ""),
            Err(OpenAiEmbeddingHttpError::Adapter(
                OpenAiEmbeddingAdapterError::EmptyModel
            ))
        ));
        let client = OpenAiEmbeddingHttpClient::new("sk-test", "text-embedding-3-small")
            .unwrap_or_else(|error| panic!("client should build: {error}"));
        let debug = format!("{client:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("sk-test"));
    }

    #[test]
    fn endpoint_url_normalizes_v1_and_full_embeddings_base() {
        let v1_base = OpenAiEmbeddingHttpClient::new("sk-test", "text-embedding-3-small")
            .unwrap_or_else(|error| panic!("client should build: {error}"))
            .with_base_url("https://api.openai.com/v1")
            .unwrap_or_else(|error| panic!("base url should parse: {error}"));
        let full_endpoint = OpenAiEmbeddingHttpClient::new("sk-test", "text-embedding-3-small")
            .unwrap_or_else(|error| panic!("client should build: {error}"))
            .with_base_url("https://gateway.example.test/openai/v1/embeddings")
            .unwrap_or_else(|error| panic!("base url should parse: {error}"));

        assert_eq!(
            embeddings_url(&v1_base.base_url)
                .unwrap_or_else(|error| panic!("url should build: {error}"))
                .as_str(),
            "https://api.openai.com/v1/embeddings"
        );
        assert_eq!(
            embeddings_url(&full_endpoint.base_url)
                .unwrap_or_else(|error| panic!("url should build: {error}"))
                .as_str(),
            "https://gateway.example.test/openai/v1/embeddings"
        );
        assert!(
            OpenAiEmbeddingHttpClient::new("sk-test", "text-embedding-3-small")
                .unwrap_or_else(|error| panic!("client should build: {error}"))
                .with_base_url("api.openai.com/v1")
                .is_err()
        );
    }

    #[test]
    fn cassette_replay_decodes_first_embedding_vector() {
        let raw = r#"{
            "object": "list",
            "data": [
                {
                    "object": "embedding",
                    "index": 0,
                    "embedding": [0.1, -0.2, 0.3]
                }
            ],
            "model": "text-embedding-3-small",
            "usage": { "prompt_tokens": 3, "total_tokens": 3 }
        }"#;
        let envelope: EmbeddingsEnvelope =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("envelope parses: {error}"));
        let embedding = parse_response("text-embedding-3-small", envelope)
            .unwrap_or_else(|error| panic!("embedding decodes: {error}"));
        assert_eq!(embedding.model_id, "text-embedding-3-small");
        assert_eq!(embedding.vector, vec![0.1, -0.2, 0.3]);
    }

    #[test]
    fn cassette_replay_errors_when_data_is_empty() {
        let envelope: EmbeddingsEnvelope = serde_json::from_str(r#"{ "data": [] }"#)
            .unwrap_or_else(|error| panic!("envelope parses: {error}"));
        let err = parse_response("text-embedding-3-small", envelope).err();
        assert!(matches!(
            err,
            Some(OpenAiEmbeddingHttpError::InvalidResponse(_))
        ));
    }

    #[test]
    fn cassette_replay_rejects_empty_or_non_finite_vectors() {
        let empty = EmbeddingsEnvelope {
            data: vec![EmbeddingData {
                embedding: Vec::new(),
            }],
        };
        let non_finite = EmbeddingsEnvelope {
            data: vec![EmbeddingData {
                embedding: vec![f32::NAN],
            }],
        };

        assert!(matches!(
            parse_response("text-embedding-3-small", empty),
            Err(OpenAiEmbeddingHttpError::Adapter(
                OpenAiEmbeddingAdapterError::EmptyVector
            ))
        ));
        assert!(matches!(
            parse_response("text-embedding-3-small", non_finite),
            Err(OpenAiEmbeddingHttpError::Adapter(
                OpenAiEmbeddingAdapterError::NonFiniteVector
            ))
        ));
    }
}
