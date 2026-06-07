# tdw-embed-local

Deterministic, offline embedding provider — the workspace's default embedder.

## Purpose

[`HashEmbeddingProvider`] implements [`tdw_embed::EmbeddingProvider`] with a pure
byte-hash computation: it folds each input byte into a fixed-dimension `f32`
vector. No network, no model files, no API key — identical input always yields an
identical vector. It is the default embedder behind `tdw-knowledge` so indexing
and search stay reproducible in CI.

It is **not** a semantic model: it exists to make the embedding/vector pipeline
runnable and deterministic offline. Swap in `tdw-embed-openai` /
`tdw-embed-google` (behind their `http` features) for real semantic embeddings.

## Feature flags

None. Dependencies are `async-trait` and `tdw-embed` (plus `tokio` as a
dev-dependency for the async tests).

## Environment variables

None. The provider is fully self-contained.

## Quickstart

```rust
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;

# async fn run() -> tdw_embed::Result<()> {
// Default: model id "local-hash-8", 8 dimensions.
let provider = HashEmbeddingProvider::default();
let embedding = provider.embed("macro research").await?;
assert_eq!(embedding.vector.len(), 8);

// Or choose your own id + dimension:
let custom = HashEmbeddingProvider::new("local-hash-16", 16)?;
# Ok(()) }
```

`HashEmbeddingProvider::new` rejects an empty model id (`EmptyModelId`) and a
zero dimension (`InvalidDimensions`).

## Example

```text
cargo run --example tdw_embed_local_basic -p tdw-embed-local
```

`examples/basic.rs` runs a real hash-embedding round-trip and asserts
determinism (same text → same vector).
