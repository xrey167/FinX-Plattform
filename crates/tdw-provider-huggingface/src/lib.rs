#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_fetcher;

#[cfg(feature = "http")]
pub use http_fetcher::HuggingFaceHttpTextGenerationFetcher;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PROVIDER_ID: &str = "huggingface";
pub const BASE_URL: &str = "https://api-inference.huggingface.co";
pub const AUTH_HEADER: &str = "Authorization";

pub type Result<T> = std::result::Result<T, HuggingFaceProviderError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InferenceRequest {
    pub provider: &'static str,
    pub model_id: String,
    pub path: String,
    pub auth_header: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HuggingFaceTextGenerationQuery {
    pub model_id: String,
    pub inputs: String,
    pub max_new_tokens: Option<u32>,
}

impl HuggingFaceTextGenerationQuery {
    pub fn new(model_id: &str, inputs: &str) -> Result<Self> {
        let inputs = inputs.trim();
        if inputs.is_empty() {
            return Err(HuggingFaceProviderError::EmptyInput);
        }
        Ok(Self {
            model_id: normalize_model_id(model_id)?,
            inputs: inputs.to_string(),
            max_new_tokens: None,
        })
    }

    pub fn with_max_new_tokens(mut self, max_new_tokens: u32) -> Result<Self> {
        if max_new_tokens == 0 {
            return Err(HuggingFaceProviderError::InvalidLimit);
        }
        self.max_new_tokens = Some(max_new_tokens);
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HuggingFaceTextGeneration {
    pub model_id: String,
    pub generated_text: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HuggingFaceProviderError {
    #[error("huggingface model id must not be empty")]
    EmptyModelId,
    #[error("huggingface model id contains unsupported path characters")]
    InvalidModelId,
    #[error("huggingface text-generation input must not be empty")]
    EmptyInput,
    #[error("huggingface text-generation token limit must be greater than zero")]
    InvalidLimit,
    #[error("huggingface token must be supplied by the caller")]
    MissingToken,
}

pub fn text_generation_request(model_id: &str, token_present: bool) -> Result<InferenceRequest> {
    if !token_present {
        return Err(HuggingFaceProviderError::MissingToken);
    }
    let model_id = normalize_model_id(model_id)?;
    Ok(InferenceRequest {
        provider: PROVIDER_ID,
        path: format!("/models/{model_id}"),
        model_id,
        auth_header: AUTH_HEADER,
    })
}

fn normalize_model_id(model_id: &str) -> Result<String> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err(HuggingFaceProviderError::EmptyModelId);
    }
    if model_id.starts_with('/')
        || model_id.ends_with('/')
        || model_id.contains("//")
        || model_id
            .split('/')
            .any(|segment| segment == "." || segment == "..")
        || !model_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | '-' | '_' | '.')
        })
    {
        return Err(HuggingFaceProviderError::InvalidModelId);
    }
    Ok(model_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_inference_request_contract_without_network_call() {
        let request = text_generation_request("mistralai/Mistral-7B", true)
            .unwrap_or_else(|error| panic!("request should build: {error}"));

        assert_eq!(request.provider, "huggingface");
        assert_eq!(request.path, "/models/mistralai/Mistral-7B");
        assert_eq!(request.auth_header, AUTH_HEADER);
        assert!(text_generation_request("model", false).is_err());
        assert!(text_generation_request("", true).is_err());
        assert!(text_generation_request("../secret", true).is_err());
    }

    #[test]
    fn builds_text_generation_query_model() {
        let query = HuggingFaceTextGenerationQuery::new("gpt2", "Hello")
            .and_then(|query| query.with_max_new_tokens(8))
            .unwrap_or_else(|error| panic!("query should build: {error}"));

        assert_eq!(query.model_id, "gpt2");
        assert_eq!(query.inputs, "Hello");
        assert_eq!(query.max_new_tokens, Some(8));
        assert!(HuggingFaceTextGenerationQuery::new("../secret", "Hello").is_err());
        assert!(HuggingFaceTextGenerationQuery::new("gpt2", " ").is_err());
        assert!(
            HuggingFaceTextGenerationQuery::new("gpt2", "Hello")
                .and_then(|query| query.with_max_new_tokens(0))
                .is_err()
        );
    }
}
