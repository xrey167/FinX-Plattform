//! Offline `tdw-embed` example: implement the `EmbeddingProvider` trait with a
//! tiny in-crate provider, produce an `Embedding`, and validate it.
//!
//! No network and no API key — it exercises the trait + DTO + validator only.
//!
//! ```text
//! cargo run --example tdw_embed_basic -p tdw-embed
//! ```

use tdw_embed::{Embedding, EmbeddingError, EmbeddingProvider, Result, validate_embedding};

/// A deterministic, offline `EmbeddingProvider` standing in for a real adapter.
struct ConstantProvider;

#[async_trait::async_trait]
impl EmbeddingProvider for ConstantProvider {
    fn model_id(&self) -> &'static str {
        "example-constant"
    }

    async fn embed(&self, text: &str) -> Result<Embedding> {
        if text.trim().is_empty() {
            return Err(EmbeddingError::EmptyInput);
        }
        Ok(Embedding {
            model_id: self.model_id().to_string(),
            vector: vec![0.1, 0.2, 0.3],
        })
    }
}

#[tokio::main]
async fn main() {
    let provider = ConstantProvider;

    let embedding = provider
        .embed("macro research note")
        .await
        .expect("embedding should succeed");

    validate_embedding(&embedding).expect("embedding should be valid");
    assert_eq!(embedding.model_id, "example-constant");
    assert_eq!(embedding.vector.len(), 3);
    println!("model:  {}", embedding.model_id);
    println!("vector: {:?}", embedding.vector);

    // Empty input is rejected by the provider's own contract.
    assert!(provider.embed("  ").await.is_err());
}
