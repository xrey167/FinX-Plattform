//! Live integration test for the Gemini Embeddings HTTP client.
//!
//! Doubly gated:
//!   - Compiled only with `--features http` (no reqwest dep otherwise).
//!   - Runs only when `TDW_GOOGLE_EMBEDDING_LIVE=1` and an API key
//!     env var is set. Cassette tests live inside the module.

#![cfg(feature = "http")]

use tdw_embed::validate_embedding;
use tdw_embed_google::GoogleEmbeddingHttpClient;

const API_KEY_ENVS: [&str; 3] = [
    "TDW_GOOGLE_EMBEDDING_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
];

fn live_enabled() -> bool {
    std::env::var("TDW_GOOGLE_EMBEDDING_LIVE").ok().as_deref() == Some("1")
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
    std::env::var("TDW_GOOGLE_EMBEDDING_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn model_id() -> String {
    std::env::var("TDW_GOOGLE_EMBEDDING_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "gemini-embedding-001".to_string())
}

#[tokio::test]
async fn live_google_embedding_returns_vector_when_env_var_set() {
    if !live_enabled() {
        eprintln!("TDW_GOOGLE_EMBEDDING_LIVE != 1; skipping live google embedding test");
        return;
    }
    let Some(key) = api_key() else {
        eprintln!("no Google embedding API key env var set; skipping live embedding test");
        return;
    };

    let mut client = GoogleEmbeddingHttpClient::new(key, model_id())
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
