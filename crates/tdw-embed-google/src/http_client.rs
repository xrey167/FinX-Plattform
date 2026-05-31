//! Real Gemini Embeddings HTTP client for G012.
//!
//! Gated by the `http` feature. Posts to Gemini's
//! `POST /v1beta/models/{model}:embedContent` endpoint with
//! `x-goog-api-key` authentication. The existing request-builder
//! helpers remain available for offline tests and higher-level
//! dispatchers.

use reqwest::{Client, Url};
use serde::Deserialize;
use tdw_embed::{Embedding, EmbeddingError, EmbeddingProvider};
use thiserror::Error;

use crate::{
    DEFAULT_BASE_URL, GoogleEmbeddingAdapterError, build_embedding_request, decode_embedding,
};

/// Production Gemini embeddings client.
#[derive(Clone)]
pub struct GoogleEmbeddingHttpClient {
    client: Client,
    base_url: Url,
    api_key: String,
    model_id: String,
}

impl std::fmt::Debug for GoogleEmbeddingHttpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GoogleEmbeddingHttpClient")
            .field("base_url", &self.base_url.as_str())
            .field("model_id", &self.model_id)
            .field("api_key", &"REDACTED")
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum GoogleEmbeddingHttpError {
    #[error("google embedding api key must not be empty")]
    MissingApiKey,
    #[error("invalid base url: {0}")]
    InvalidBaseUrl(String),
    #[error("google embedding adapter error: {0}")]
    Adapter(#[from] GoogleEmbeddingAdapterError),
    #[error("google embedding http {status}: {body}")]
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

impl GoogleEmbeddingHttpClient {
    /// Construct a client for an authenticated Gemini embedding
    /// endpoint. Defaults to
    /// `https://generativelanguage.googleapis.com/v1beta`.
    pub fn new(
        api_key: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, GoogleEmbeddingHttpError> {
        let api_key = normalize_api_key(api_key.into())?;
        let model_id = normalize_model(model_id.into())?;
        let base_url = parse_base_url(DEFAULT_BASE_URL)?;
        let client = Client::builder()
            .user_agent("tdw-embed-google/0.1")
            .build()
            .map_err(|error| GoogleEmbeddingHttpError::ClientBuild(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            api_key,
            model_id,
        })
    }

    /// Override the base URL. Supplying the Gemini API root, a
    /// versioned `/v1beta` base, or a full `:embedContent` endpoint
    /// is accepted.
    pub fn with_base_url(
        mut self,
        base_url: impl Into<String>,
    ) -> Result<Self, GoogleEmbeddingHttpError> {
        self.base_url = parse_base_url(&base_url.into())?;
        Ok(self)
    }

    /// Gemini embedding model id this client is configured for.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Post `input` to Gemini's embedContent endpoint and decode the
    /// returned vector into the workspace [`Embedding`] contract.
    pub async fn embed(&self, input: &str) -> Result<Embedding, GoogleEmbeddingHttpError> {
        let request = build_embedding_request(&self.model_id, input, true)?;
        let response = self
            .client
            .post(embed_content_url(&self.base_url, &self.model_id)?)
            .header("x-goog-api-key", &self.api_key)
            .header("content-type", "application/json")
            .json(&request.body)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(GoogleEmbeddingHttpError::Http { status, body });
        }
        let envelope: EmbedContentEnvelope = response.json().await?;
        parse_response(&self.model_id, envelope)
    }
}

/// Bridge the real Gemini HTTP client onto the workspace
/// [`EmbeddingProvider`] trait so the knowledge index can drive it
/// behind `Arc<dyn EmbeddingProvider>`. The crate-specific
/// [`GoogleEmbeddingHttpError`] is flattened onto
/// [`EmbeddingError::Provider`], preserving its message.
#[async_trait::async_trait]
impl EmbeddingProvider for GoogleEmbeddingHttpClient {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn embed(&self, text: &str) -> tdw_embed::Result<Embedding> {
        GoogleEmbeddingHttpClient::embed(self, text)
            .await
            .map_err(|error| EmbeddingError::Provider(error.to_string()))
    }
}

fn normalize_api_key(api_key: String) -> Result<String, GoogleEmbeddingHttpError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(GoogleEmbeddingHttpError::MissingApiKey);
    }
    Ok(api_key.to_string())
}

fn normalize_model(model_id: String) -> Result<String, GoogleEmbeddingHttpError> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err(GoogleEmbeddingAdapterError::EmptyModel.into());
    }
    let model_id = model_id.strip_prefix("models/").unwrap_or(model_id);
    if model_id.is_empty() {
        return Err(GoogleEmbeddingAdapterError::EmptyModel.into());
    }
    Ok(model_id.to_string())
}

fn parse_base_url(base_url: &str) -> Result<Url, GoogleEmbeddingHttpError> {
    if base_url
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(GoogleEmbeddingHttpError::InvalidBaseUrl(
            "base url contains whitespace or control characters".to_string(),
        ));
    }
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err(GoogleEmbeddingHttpError::InvalidBaseUrl(
            "base url must start with http:// or https://".to_string(),
        ));
    }
    Url::parse(base_url)
        .map_err(|error| GoogleEmbeddingHttpError::InvalidBaseUrl(error.to_string()))
}

fn embed_content_url(base_url: &Url, model_id: &str) -> Result<Url, GoogleEmbeddingHttpError> {
    let mut url = base_url.clone();
    let path = url.path().trim_end_matches('/');
    let path = if path.ends_with(":embedContent") {
        path.to_string()
    } else if path.ends_with("/v1beta") || path.ends_with("/v1") {
        format!("{path}/models/{model_id}:embedContent")
    } else if path.is_empty() {
        format!("/v1beta/models/{model_id}:embedContent")
    } else {
        format!("{path}/models/{model_id}:embedContent")
    };
    url.set_path(&path);
    url.set_query(None);
    Ok(url)
}

#[derive(Deserialize)]
pub(crate) struct EmbedContentEnvelope {
    #[serde(default)]
    embedding: Option<ContentEmbedding>,
}

#[derive(Deserialize)]
pub(crate) struct ContentEmbedding {
    #[serde(default)]
    values: Vec<f32>,
}

/// Parse a Gemini embedContent envelope into the workspace
/// [`Embedding`] contract. Extracted so cassette tests can exercise
/// decoder behavior without touching the network.
pub(crate) fn parse_response(
    model_id: &str,
    envelope: EmbedContentEnvelope,
) -> Result<Embedding, GoogleEmbeddingHttpError> {
    let vector = envelope
        .embedding
        .map(|embedding| embedding.values)
        .ok_or_else(|| {
            GoogleEmbeddingHttpError::InvalidResponse(
                "google embedding response had no embedding field".to_string(),
            )
        })?;
    decode_embedding(model_id, vector).map_err(GoogleEmbeddingHttpError::Adapter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_constructor_rejects_empty_key_and_model_and_redacts_debug() {
        assert!(matches!(
            GoogleEmbeddingHttpClient::new("", "gemini-embedding-001"),
            Err(GoogleEmbeddingHttpError::MissingApiKey)
        ));
        assert!(matches!(
            GoogleEmbeddingHttpClient::new("AIza-test", ""),
            Err(GoogleEmbeddingHttpError::Adapter(
                GoogleEmbeddingAdapterError::EmptyModel
            ))
        ));
        let client = GoogleEmbeddingHttpClient::new("AIza-test", "gemini-embedding-001")
            .unwrap_or_else(|error| panic!("client should build: {error}"));
        let debug = format!("{client:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("AIza-test"));
    }

    #[test]
    fn endpoint_url_normalizes_api_root_version_and_full_embed_endpoint() {
        let api_root = GoogleEmbeddingHttpClient::new("AIza-test", "gemini-embedding-001")
            .unwrap_or_else(|error| panic!("client should build: {error}"))
            .with_base_url("https://generativelanguage.googleapis.com")
            .unwrap_or_else(|error| panic!("base url should parse: {error}"));
        let version_base = GoogleEmbeddingHttpClient::new("AIza-test", "gemini-embedding-001")
            .unwrap_or_else(|error| panic!("client should build: {error}"))
            .with_base_url("https://generativelanguage.googleapis.com/v1beta")
            .unwrap_or_else(|error| panic!("base url should parse: {error}"));
        let full_endpoint = GoogleEmbeddingHttpClient::new("AIza-test", "gemini-embedding-001")
            .unwrap_or_else(|error| panic!("client should build: {error}"))
            .with_base_url(
                "https://gateway.example.test/v1beta/models/custom-embedding:embedContent",
            )
            .unwrap_or_else(|error| panic!("base url should parse: {error}"));

        assert_eq!(
            embed_content_url(&api_root.base_url, api_root.model_id())
                .unwrap_or_else(|error| panic!("url should build: {error}"))
                .as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:embedContent"
        );
        assert_eq!(
            embed_content_url(&version_base.base_url, version_base.model_id())
                .unwrap_or_else(|error| panic!("url should build: {error}"))
                .as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:embedContent"
        );
        assert_eq!(
            embed_content_url(&full_endpoint.base_url, full_endpoint.model_id())
                .unwrap_or_else(|error| panic!("url should build: {error}"))
                .as_str(),
            "https://gateway.example.test/v1beta/models/custom-embedding:embedContent"
        );
        assert_eq!(
            GoogleEmbeddingHttpClient::new("AIza-test", "models/gemini-embedding-001")
                .unwrap_or_else(|error| panic!("client should build: {error}"))
                .model_id(),
            "gemini-embedding-001"
        );
        assert!(
            GoogleEmbeddingHttpClient::new("AIza-test", "gemini-embedding-001")
                .unwrap_or_else(|error| panic!("client should build: {error}"))
                .with_base_url("generativelanguage.googleapis.com/v1beta")
                .is_err()
        );
    }

    #[test]
    fn cassette_replay_decodes_embedding_vector() {
        let raw = r#"{
            "embedding": {
                "values": [0.1, -0.2, 0.3]
            },
            "usageMetadata": {
                "promptTokenCount": 4
            }
        }"#;
        let envelope: EmbedContentEnvelope =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("envelope parses: {error}"));
        let embedding = parse_response("gemini-embedding-001", envelope)
            .unwrap_or_else(|error| panic!("embedding decodes: {error}"));
        assert_eq!(embedding.model_id, "gemini-embedding-001");
        assert_eq!(embedding.vector, vec![0.1, -0.2, 0.3]);
    }

    #[test]
    fn cassette_replay_errors_when_embedding_is_missing() {
        let envelope: EmbedContentEnvelope = serde_json::from_str(r#"{ "usageMetadata": {} }"#)
            .unwrap_or_else(|error| panic!("envelope parses: {error}"));
        let err = parse_response("gemini-embedding-001", envelope).err();
        assert!(matches!(
            err,
            Some(GoogleEmbeddingHttpError::InvalidResponse(_))
        ));
    }

    #[test]
    fn cassette_replay_rejects_empty_or_non_finite_vectors() {
        let empty = EmbedContentEnvelope {
            embedding: Some(ContentEmbedding { values: Vec::new() }),
        };
        let non_finite = EmbedContentEnvelope {
            embedding: Some(ContentEmbedding {
                values: vec![f32::NAN],
            }),
        };

        assert!(matches!(
            parse_response("gemini-embedding-001", empty),
            Err(GoogleEmbeddingHttpError::Adapter(
                GoogleEmbeddingAdapterError::EmptyVector
            ))
        ));
        assert!(matches!(
            parse_response("gemini-embedding-001", non_finite),
            Err(GoogleEmbeddingHttpError::Adapter(
                GoogleEmbeddingAdapterError::NonFiniteVector
            ))
        ));
    }
}
