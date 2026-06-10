//! Contract + offline error-path tests for the local `BERT` embedder (B6).
//!
//! The LIVE leg self-skips unless `TDW_LOCAL_MODEL_DIR` points at a real model
//! directory, mirroring the Bolt conformance precedent in
//! `crates/tdw-storage-graph/tests/conformance.rs`. The OFFLINE error-path legs
//! always run: they only need the `model` feature, not a model on disk.

#![cfg(feature = "model")]
#![forbid(unsafe_code)]

use std::path::PathBuf;

use tdw_embed::{EmbeddingError, EmbeddingProvider, validate_batch, validate_embedding};
use tdw_embed_local::LocalModelEmbeddingProvider;

/// LIVE leg: runs the full provider contract against a real local model.
///
/// Self-skips (prints a note, returns `Ok`) when `TDW_LOCAL_MODEL_DIR` is unset
/// so the default test run stays green and offline.
#[tokio::test(flavor = "multi_thread")]
async fn live_local_model_contract() -> Result<(), EmbeddingError> {
    let Ok(model_dir) = std::env::var("TDW_LOCAL_MODEL_DIR") else {
        eprintln!("TDW_LOCAL_MODEL_DIR unset; skipping live local-model leg");
        return Ok(());
    };

    let provider = LocalModelEmbeddingProvider::from_dir(&model_dir)?;
    assert!(
        !provider.model_id().is_empty(),
        "model_id must be non-empty"
    );

    let text = "macro research note on rate-cut expectations";
    let first = provider.embed(text).await?;
    assert_eq!(first.model_id, provider.model_id());
    validate_embedding(&first)?;
    assert!(!first.vector.is_empty(), "vector must be non-empty");
    assert!(
        first.vector.iter().all(|value| value.is_finite()),
        "vector must be finite"
    );

    // Determinism: identical text twice yields a bit-identical vector.
    let second = provider.embed(text).await?;
    assert_eq!(
        first.vector, second.vector,
        "embedding must be deterministic"
    );

    // embed_batch parity via the shared validator.
    let texts = vec![text.to_string(), "a different sentence".to_string()];
    let batch = provider.embed_batch(&texts).await?;
    validate_batch(&texts, &batch)?;
    assert_eq!(
        batch[0].vector, first.vector,
        "batch[0] must match single embed"
    );

    // Empty / whitespace input errors.
    assert!(matches!(
        provider.embed("   ").await,
        Err(EmbeddingError::EmptyInput)
    ));

    Ok(())
}

/// OFFLINE: `from_dir` on a missing directory errors loudly.
#[test]
fn from_dir_missing_directory_errors() {
    let missing = std::env::temp_dir().join("tdw-embed-local-does-not-exist-b6");
    // Best-effort cleanup of any stale fixture from a prior crashed run.
    let _ = std::fs::remove_dir_all(&missing);

    let result = LocalModelEmbeddingProvider::from_dir(&missing);
    assert!(
        matches!(result, Err(EmbeddingError::Provider(_))),
        "missing directory must surface a Provider error, got {result:?}"
    );
}

/// OFFLINE: `from_dir` on a directory missing `tokenizer.json` errors loudly.
#[test]
fn from_dir_missing_tokenizer_errors() {
    let fixture: PathBuf = std::env::temp_dir().join("tdw-embed-local-fixture-no-tokenizer-b6");
    let _ = std::fs::remove_dir_all(&fixture);
    std::fs::create_dir_all(&fixture).expect("create fixture dir");

    // A minimal-but-parseable BERT config so loading reaches the tokenizer step.
    let config = r#"{
        "hidden_size": 8,
        "intermediate_size": 16,
        "max_position_embeddings": 16,
        "num_attention_heads": 2,
        "num_hidden_layers": 1,
        "vocab_size": 32,
        "type_vocab_size": 2,
        "layer_norm_eps": 1e-12,
        "hidden_act": "gelu",
        "pad_token_id": 0
    }"#;
    std::fs::write(fixture.join("config.json"), config).expect("write config.json");

    let result = LocalModelEmbeddingProvider::from_dir(&fixture);

    let _ = std::fs::remove_dir_all(&fixture);

    assert!(
        matches!(result, Err(EmbeddingError::Provider(_))),
        "missing tokenizer.json must surface a Provider error, got {result:?}"
    );
}
