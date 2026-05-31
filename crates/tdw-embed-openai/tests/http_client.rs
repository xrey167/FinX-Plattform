//! Live integration test for the `OpenAI` Embeddings HTTP client.
//!
//! Doubly gated:
//!   - Compiled only with `--features http` (no reqwest dep otherwise).
//!   - Runs only when `TDW_OPENAI_EMBEDDING_LIVE=1` and an API key
//!     env var is set. Cassette tests live inside the module.

#![cfg(feature = "http")]

use tdw_embed::{EmbeddingProvider, validate_embedding};
use tdw_embed_openai::OpenAiEmbeddingHttpClient;

const API_KEY_ENVS: [&str; 3] = [
    "TDW_OPENAI_EMBEDDING_API_KEY",
    "TDW_EMBED_TEST_KEY",
    "OPENAI_API_KEY",
];

fn live_enabled() -> bool {
    std::env::var("TDW_OPENAI_EMBEDDING_LIVE").ok().as_deref() == Some("1")
}

fn api_key() -> Option<String> {
    API_KEY_ENVS.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn base_url() -> Option<String> {
    std::env::var("TDW_OPENAI_EMBEDDING_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn model_id() -> String {
    std::env::var("TDW_OPENAI_EMBEDDING_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "text-embedding-3-small".to_string())
}

#[tokio::test]
async fn live_openai_embedding_returns_vector_when_env_var_set() {
    if !live_enabled() {
        eprintln!("TDW_OPENAI_EMBEDDING_LIVE != 1; skipping live openai embedding test");
        return;
    }
    let Some(key) = api_key() else {
        eprintln!("no OpenAI embedding API key env var set; skipping live embedding test");
        return;
    };

    let mut client = OpenAiEmbeddingHttpClient::new(key, model_id())
        .unwrap_or_else(|error| panic!("client must build: {error}"));
    if let Some(url) = base_url() {
        client = client
            .with_base_url(url)
            .unwrap_or_else(|error| panic!("base url must parse: {error}"));
    }

    let embedding = client
        .embed("short macro note")
        .await
        .unwrap_or_else(|error| panic!("live embedding request must succeed: {error}"));

    assert_eq!(embedding.model_id, client.model_id());
    assert!(
        embedding.vector.len() >= 2,
        "live embedding vector unexpectedly small: {embedding:?}"
    );
    validate_embedding(&embedding)
        .unwrap_or_else(|error| panic!("live embedding must validate: {error}"));
}

/// The same live request driven through the workspace
/// [`EmbeddingProvider`] trait bridge (the path the knowledge index
/// uses behind `Arc<dyn EmbeddingProvider>`), proving the A2 adapter
/// returns a non-empty vector of the configured model. Env-gated on
/// `TDW_OPENAI_EMBEDDING_LIVE=1` plus an API key
/// (`TDW_OPENAI_EMBEDDING_API_KEY`, `TDW_EMBED_TEST_KEY`, or
/// `OPENAI_API_KEY`); early-returns when unset.
#[tokio::test]
async fn live_openai_embedding_via_provider_trait_returns_vector_when_env_var_set() {
    if !live_enabled() {
        eprintln!("TDW_OPENAI_EMBEDDING_LIVE != 1; skipping live provider-trait embedding test");
        return;
    }
    let Some(key) = api_key() else {
        eprintln!("no OpenAI embedding API key env var set; skipping live provider-trait test");
        return;
    };

    let mut client = OpenAiEmbeddingHttpClient::new(key, model_id())
        .unwrap_or_else(|error| panic!("client must build: {error}"));
    if let Some(url) = base_url() {
        client = client
            .with_base_url(url)
            .unwrap_or_else(|error| panic!("base url must parse: {error}"));
    }

    let provider: &dyn EmbeddingProvider = &client;
    let expected_model = provider.model_id().to_string();
    let embedding = provider
        .embed("short macro note")
        .await
        .unwrap_or_else(|error| panic!("live provider-trait embedding must succeed: {error}"));

    assert_eq!(embedding.model_id, expected_model);
    assert!(
        !embedding.vector.is_empty(),
        "live provider-trait embedding vector must be non-empty: {embedding:?}"
    );
    validate_embedding(&embedding)
        .unwrap_or_else(|error| panic!("live provider-trait embedding must validate: {error}"));
}
