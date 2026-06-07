# tdw-embed — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`:

| Item | Role |
| --- | --- |
| `EmbeddingError` / `Result<T>` | Error enum + alias: `EmptyModelId`, `EmptyInput`, `InvalidDimensions`, `EmptyVector`, `NonFiniteVector`, `Provider(String)`. |
| `Embedding` | `{ model_id: String, vector: Vec<f32> }`. |
| `EmbeddingProvider` | The async trait. |
| `validate_embedding` | Vector hygiene check. |

## Trait contract: `EmbeddingProvider`

```rust
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn model_id(&self) -> &str;
    async fn embed(&self, text: &str) -> Result<Embedding>;
}
```

- **Async**, unlike `tdw_llm::LanguageModel` — embedding providers are commonly
  network-backed, so the trait is `async fn` (via `async-trait`). The offline
  `tdw-embed-local::HashEmbeddingProvider` satisfies it with a pure computation.
- `Send + Sync` so a provider lives behind `Arc<dyn EmbeddingProvider>` and is
  shared (see `tdw-knowledge::KnowledgeIndex`).
- The returned `Embedding` carries its `model_id` so downstream consumers can
  namespace vector collections per model (different dimensions must never share a
  collection).

## Validation contract

`validate_embedding`:

1. `model_id` must be non-empty after trim.
2. `vector` must be non-empty.
3. Every component must be finite (no `NaN`/`inf`).

Provider adapters call this (or equivalent provider-specific checks) before
handing an embedding to a vector store, so a malformed vector never silently
corrupts an index.

## Embedding flow

```
text ─▶ EmbeddingProvider::embed (per-provider)
          ├─ local: pure hash computation (offline)
          └─ http:  POST provider /embeddings → parse vector
        ─▶ Embedding { model_id, vector }
          └─ validate_embedding before storage
```

## Offline / cassette-test design

The crate test uses an in-module `ConstantProvider`; there is no network, so the
default test run is offline and deterministic. Provider adapters that talk HTTP
keep their request-builder + vector-decoder as pure, unit-tested functions
(`build_embedding_request` / `decode_embedding`) and gate the live network test
behind a feature plus a `TDW_*_LIVE` env var.
