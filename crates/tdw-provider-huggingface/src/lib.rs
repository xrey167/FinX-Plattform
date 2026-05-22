#![forbid(unsafe_code)]

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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HuggingFaceProviderError {
    #[error("huggingface model id must not be empty")]
    EmptyModelId,
    #[error("huggingface model id contains unsupported path characters")]
    InvalidModelId,
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
}
