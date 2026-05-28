#![forbid(unsafe_code)]

use serde_json::{Value, json};
use tdw_embed::Embedding;
use thiserror::Error;

#[cfg(feature = "http")]
pub mod http_client;
#[cfg(feature = "http")]
pub use http_client::{GoogleEmbeddingHttpClient, GoogleEmbeddingHttpError};

pub const PROVIDER_ID: &str = "google";
pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

pub type Result<T> = std::result::Result<T, GoogleEmbeddingAdapterError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbeddingHttpRequest {
    pub provider: &'static str,
    pub base_url: String,
    pub path: String,
    pub api_key_required: bool,
    pub body: Value,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GoogleEmbeddingAdapterError {
    #[error("google embedding model id must not be empty")]
    EmptyModel,
    #[error("google embedding input must not be empty")]
    EmptyInput,
    #[error("google api key must be supplied by the caller")]
    MissingApiKey,
    #[error("google embedding vector must not be empty")]
    EmptyVector,
    #[error("google embedding vector contains a non-finite value")]
    NonFiniteVector,
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn build_embedding_request(
    model: &str,
    input: &str,
    api_key_present: bool,
) -> Result<EmbeddingHttpRequest> {
    if !api_key_present {
        return Err(GoogleEmbeddingAdapterError::MissingApiKey);
    }
    let model = normalize_component(model, GoogleEmbeddingAdapterError::EmptyModel)?;
    let input = normalize_component(input, GoogleEmbeddingAdapterError::EmptyInput)?;
    Ok(EmbeddingHttpRequest {
        provider: PROVIDER_ID,
        base_url: DEFAULT_BASE_URL.to_string(),
        path: format!("/models/{model}:embedContent"),
        api_key_required: true,
        body: json!({
            "model": format!("models/{model}"),
            "content": {
                "parts": [{ "text": input }]
            }
        }),
    })
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn decode_embedding(model: &str, vector: Vec<f32>) -> Result<Embedding> {
    let model = normalize_component(model, GoogleEmbeddingAdapterError::EmptyModel)?;
    if vector.is_empty() {
        return Err(GoogleEmbeddingAdapterError::EmptyVector);
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(GoogleEmbeddingAdapterError::NonFiniteVector);
    }
    Ok(Embedding {
        model_id: model,
        vector,
    })
}

fn normalize_component(value: &str, error: GoogleEmbeddingAdapterError) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(error);
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_request_and_decodes_vector_without_network_call() {
        let request = build_embedding_request("text-embedding-004", "macro note", true)
            .unwrap_or_else(|error| panic!("request should build: {error}"));
        let embedding = decode_embedding("text-embedding-004", vec![0.1, 0.2])
            .unwrap_or_else(|error| panic!("embedding should decode: {error}"));

        assert_eq!(request.provider, "google");
        assert_eq!(request.path, "/models/text-embedding-004:embedContent");
        assert_eq!(request.body["model"], "models/text-embedding-004");
        assert_eq!(request.body["content"]["parts"][0]["text"], "macro note");
        assert_eq!(embedding.vector.len(), 2);
        assert!(build_embedding_request("model", "input", false).is_err());
        assert!(build_embedding_request("", "input", true).is_err());
        assert!(build_embedding_request("model", "", true).is_err());
        assert!(decode_embedding("model", Vec::new()).is_err());
        assert!(decode_embedding("model", vec![f32::INFINITY]).is_err());
    }
}
