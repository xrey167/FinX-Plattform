//! Offline `tdw-embed-local` example: a real hash-embedding round-trip proving
//! determinism — the same text always yields the same vector. No network.
//!
//! ```text
//! cargo run --example tdw_embed_local_basic -p tdw-embed-local
//! ```

use tdw_embed::{EmbeddingProvider, validate_embedding};
use tdw_embed_local::HashEmbeddingProvider;

#[tokio::main]
async fn main() {
    let provider = HashEmbeddingProvider::default();

    let first = provider
        .embed("macro research")
        .await
        .expect("embedding should succeed");
    let second = provider
        .embed("macro research")
        .await
        .expect("embedding should succeed");

    // Deterministic: identical input → identical vector.
    assert_eq!(first, second);
    assert_eq!(first.model_id, "local-hash-8");
    assert_eq!(first.vector.len(), 8);
    validate_embedding(&first).expect("embedding should be valid");

    println!("model:  {}", first.model_id);
    println!("vector: {:?}", first.vector);

    // A different model id + dimension is honored.
    let custom =
        HashEmbeddingProvider::new("local-hash-16", 16).expect("custom provider should build");
    let wide = custom.embed("macro research").await.expect("embed");
    assert_eq!(wide.vector.len(), 16);
    println!("custom dims: {}", wide.vector.len());
}
