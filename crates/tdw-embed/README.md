# tdw-embed

Provider-agnostic embedding contract for the TDW workspace.

## Purpose

`tdw-embed` defines the `async` trait and DTO every embedding provider
(`tdw-embed-local`, `tdw-embed-openai`, `tdw-embed-google`, …) implements, plus a
vector validator. It owns no network code and pulls in no HTTP dependency, so it
compiles and tests fully offline.

Core surface:

- [`EmbeddingProvider`] — `fn model_id(&self) -> &str` and
  `async fn embed(&self, text: &str) -> Result<Embedding>`.
- [`Embedding`] — `{ model_id, vector: Vec<f32> }`.
- [`EmbeddingError`] — the shared error enum.
- [`validate_embedding`] — non-empty model id + non-empty, all-finite vector.

## Feature flags

None. Dependencies are `async-trait`, `serde`, `thiserror` (and `tokio` only as a
dev-dependency for the trait test). The `http` feature lives on the provider
adapter crates.

## Environment variables

None are read here. API keys (`OPENAI_API_KEY`, `GEMINI_API_KEY`, …) and the
`TDW_*_LIVE` gates are consumed by the provider adapters.

## Quickstart

```rust
use tdw_embed::{Embedding, EmbeddingError, EmbeddingProvider, Result};

struct ConstantProvider;

#[async_trait::async_trait]
impl EmbeddingProvider for ConstantProvider {
    fn model_id(&self) -> &str { "constant" }
    async fn embed(&self, text: &str) -> Result<Embedding> {
        if text.is_empty() { return Err(EmbeddingError::EmptyInput); }
        Ok(Embedding { model_id: self.model_id().to_string(), vector: vec![1.0] })
    }
}
```

## Example

```text
cargo run --example tdw_embed_basic -p tdw-embed
```

`examples/basic.rs` defines a tiny offline `EmbeddingProvider`, produces an
`Embedding`, and validates it — no network.

## Related crates

- `tdw-embed-local` — deterministic hash embedder (no network, default backend).
- `tdw-embed-openai` / `tdw-embed-google` — request builders + `http` clients.
- `tdw-knowledge` — indexes documents via an `EmbeddingProvider` + vector engine.
