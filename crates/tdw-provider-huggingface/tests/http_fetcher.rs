//! Tests for the real HuggingFace text-generation HTTP fetcher.
//!
//! Gated by `--features http` (no reqwest dep otherwise).
//!
//! Cassette tests always run under the feature and parse recorded
//! inference response shapes. The live test is additionally gated by
//! `TDW_HUGGINGFACE_LIVE=1` and requires an HF token env var.

#![cfg(feature = "http")]

use bytes::Bytes;
use serde_json::json;
use tdw_core::{Credentials, Fetcher};
use tdw_provider_huggingface::{
    HuggingFaceHttpTextGenerationFetcher, HuggingFaceTextGenerationQuery,
};

fn sample_query() -> HuggingFaceTextGenerationQuery {
    HuggingFaceHttpTextGenerationFetcher::transform_query(json!({
        "model_id": "gpt2",
        "inputs": "The market opened",
        "max_new_tokens": 8
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"))
}

fn cassette_bytes() -> Bytes {
    Bytes::from(
        json!([
            {
                "generated_text": "The market opened higher after earnings."
            }
        ])
        .to_string()
        .into_bytes(),
    )
}

#[test]
fn cassette_replay_decodes_huggingface_generation_array() {
    let fetcher = HuggingFaceHttpTextGenerationFetcher::default();
    let query = sample_query();
    let rows = fetcher
        .transform_data(&query, cassette_bytes())
        .unwrap_or_else(|error| panic!("transform_data must succeed: {error}"));

    assert_eq!(rows.len(), 1, "rows={rows:#?}");
    assert_eq!(rows[0].model_id, "gpt2");
    assert_eq!(
        rows[0].generated_text,
        "The market opened higher after earnings."
    );
}

#[test]
fn cassette_replay_surfaces_huggingface_error_envelope() {
    let fetcher = HuggingFaceHttpTextGenerationFetcher::default();
    let query = sample_query();
    let envelope = Bytes::from(
        json!({
            "error": "Model gpt2 is currently loading"
        })
        .to_string()
        .into_bytes(),
    );
    let err = fetcher
        .transform_data(&query, envelope)
        .expect_err("error envelope must be propagated");

    assert!(err.to_string().contains("huggingface api error"));
}

#[test]
fn transform_query_normalizes_model_and_rejects_path_traversal() {
    let query = HuggingFaceHttpTextGenerationFetcher::transform_query(json!({
        "model": "gpt2",
        "prompt": "Hello"
    }))
    .unwrap_or_else(|error| panic!("query should transform: {error}"));

    assert_eq!(query.model_id, "gpt2");
    assert!(
        HuggingFaceHttpTextGenerationFetcher::transform_query(json!({
            "model_id": "../secret",
            "inputs": "Hello"
        }))
        .is_err()
    );
}

#[tokio::test]
async fn live_huggingface_returns_generation_when_env_vars_set() {
    if std::env::var("TDW_HUGGINGFACE_LIVE").ok().as_deref() != Some("1") {
        eprintln!("TDW_HUGGINGFACE_LIVE != 1; skipping live HuggingFace integration test");
        return;
    }

    let fetcher = HuggingFaceHttpTextGenerationFetcher::default();
    let query = sample_query();
    let raw = fetcher
        .extract_data(&query, &Credentials::default())
        .await
        .unwrap_or_else(|error| panic!("live extract_data must succeed: {error}"));
    let rows = fetcher
        .transform_data(&query, raw)
        .unwrap_or_else(|error| panic!("live transform_data must succeed: {error}"));

    assert!(
        !rows.is_empty(),
        "live response must include one generation row"
    );
    assert_eq!(rows[0].model_id, "gpt2");
}
