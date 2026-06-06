//! Real HuggingFace text-generation backend for `tdw_core::Fetcher`.
//!
//! Gated by the `http` feature. Talks to HuggingFace's inference
//! `/models/{model_id}` endpoint directly via `reqwest`. Live calls
//! require an HF token env var; the live integration test is
//! additionally gated by `TDW_HUGGINGFACE_LIVE=1` so unattended CI
//! stays offline.

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_core::{Credentials, Error, Fetcher, Result};

use crate::{
    AUTH_HEADER, BASE_URL, HuggingFaceTextGeneration, HuggingFaceTextGenerationQuery,
    text_generation_request,
};

const TOKEN_ENVS: [&str; 3] = ["HF_TOKEN", "HUGGINGFACE_API_TOKEN", "HF_API_TOKEN"];
const USER_AGENT: &str = "tdw-provider-huggingface/0.1";

tdw_core::provider_fetcher_struct!(
    /// Production HuggingFace text-generation fetcher.
    pub HuggingFaceHttpTextGenerationFetcher,
    BASE_URL
);

#[derive(Deserialize)]
struct HuggingFaceGeneration {
    generated_text: String,
}

#[derive(Deserialize)]
struct HuggingFaceErrorEnvelope {
    error: String,
}

#[async_trait]
impl Fetcher<HuggingFaceTextGenerationQuery, HuggingFaceTextGeneration>
    for HuggingFaceHttpTextGenerationFetcher
{
    const PROVIDER: &'static str = "huggingface";
    const ENDPOINT: &'static str = "text_generation";

    fn transform_query(params: Value) -> Result<HuggingFaceTextGenerationQuery> {
        let model_id = params
            .get("model_id")
            .or_else(|| params.get("model"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("huggingface model_id must be a string".to_string())
            })?;
        let inputs = params
            .get("inputs")
            .or_else(|| params.get("prompt"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidQuery("huggingface inputs must be a string".to_string())
            })?;
        let max_new_tokens = params
            .get("max_new_tokens")
            .and_then(Value::as_u64)
            .map(u32::try_from)
            .transpose()
            .map_err(|error| {
                Error::InvalidQuery(format!("huggingface max_new_tokens too large: {error}"))
            })?;

        let mut query = HuggingFaceTextGenerationQuery::new(model_id, inputs)
            .map_err(|error| Error::InvalidQuery(error.to_string()))?;
        if let Some(max_new_tokens) = max_new_tokens {
            query = query
                .with_max_new_tokens(max_new_tokens)
                .map_err(|error| Error::InvalidQuery(error.to_string()))?;
        }
        Ok(query)
    }

    async fn extract_data(
        &self,
        query: &HuggingFaceTextGenerationQuery,
        _creds: &Credentials,
    ) -> Result<Bytes> {
        text_generation_request(&query.model_id, true)
            .map_err(|error| Error::Provider(error.to_string()))?;
        let token = hf_token().ok_or_else(|| {
            Error::Provider(format!(
                "huggingface token env must be one of {}",
                TOKEN_ENVS.join(", ")
            ))
        })?;
        let endpoint = format!(
            "{}/models/{}",
            self.base_url().trim_end_matches('/'),
            query.model_id
        );
        let mut body = json!({ "inputs": query.inputs });
        if let Some(max_new_tokens) = query.max_new_tokens {
            body["parameters"] = json!({ "max_new_tokens": max_new_tokens });
        }

        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|error| Error::Provider(format!("huggingface client: {error}")))?;
        let response = client
            .post(&endpoint)
            .header(AUTH_HEADER, format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .map_err(|error| Error::Provider(format!("huggingface extract_data: {error}")))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::Provider(format!(
                "huggingface extract_data returned {status}: {body}"
            )));
        }
        response
            .bytes()
            .await
            .map_err(|error| Error::Provider(format!("huggingface read body: {error}")))
    }

    fn transform_data(
        &self,
        query: &HuggingFaceTextGenerationQuery,
        raw: Bytes,
    ) -> Result<Vec<HuggingFaceTextGeneration>> {
        if let Ok(error) = serde_json::from_slice::<HuggingFaceErrorEnvelope>(&raw) {
            return Err(Error::Provider(format!(
                "huggingface api error: {}",
                error.error
            )));
        }
        if let Ok(generations) = serde_json::from_slice::<Vec<HuggingFaceGeneration>>(&raw) {
            return Ok(generations
                .into_iter()
                .map(|generation| HuggingFaceTextGeneration {
                    model_id: query.model_id.clone(),
                    generated_text: generation.generated_text,
                })
                .collect());
        }
        let generation: HuggingFaceGeneration = serde_json::from_slice(&raw)
            .map_err(|error| Error::Provider(format!("huggingface parse_json: {error}")))?;
        Ok(vec![HuggingFaceTextGeneration {
            model_id: query.model_id.clone(),
            generated_text: generation.generated_text,
        }])
    }
}

fn hf_token() -> Option<String> {
    TOKEN_ENVS.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}
