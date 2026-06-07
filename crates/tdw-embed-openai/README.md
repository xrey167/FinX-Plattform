# tdw-embed-openai

OpenAI Embeddings adapter for the `tdw-embed` `EmbeddingProvider` contract.

## Purpose

Two layers in one crate:

1. **Offline request builder / decoder** — [`build_embedding_request`] produces
   the exact JSON body posted to OpenAI's `/embeddings` endpoint, and
   [`decode_embedding`] turns a returned `Vec<f32>` into a validated
   `tdw_embed::Embedding`. Both are pure and network-free.
2. **Real HTTP client** (`http` feature) — [`OpenAiEmbeddingHttpClient`] posts to
   `POST /v1/embeddings` with `reqwest` and also implements
   `tdw_embed::EmbeddingProvider` so it can drive `tdw-knowledge` behind
   `Arc<dyn EmbeddingProvider>`.

Constants: `PROVIDER_ID = "openai"`, `DEFAULT_BASE_URL = "https://api.openai.com/v1"`,
`EMBEDDINGS_PATH = "/embeddings"`.

## Feature flags

| Feature | Effect |
| --- | --- |
| `http` | Compiles [`OpenAiEmbeddingHttpClient`] and pulls in `reqwest`, `serde`, `tokio`, `async-trait`. **Off by default** so the workspace test set stays offline. |

The request builder / decoder are always available; only the HTTP client is
gated.

## Environment variables

The crate reads no env vars directly. The env-gated **live** integration test
runs only when:

| Variable | Meaning |
| --- | --- |
| `TDW_OPENAI_EMBEDDING_LIVE=1` | Opt in to the live network test. |
| `TDW_OPENAI_EMBEDDING_API_KEY` or `OPENAI_API_KEY` | The API key for the request. |

With these unset the live test early-returns cleanly (no network).

## Quickstart

Offline builder + decoder (no feature, no key — `api_key_present` is just a flag,
no real key is read):

```rust
use tdw_embed_openai::{build_embedding_request, decode_embedding};

let request = build_embedding_request("text-embedding-3-small", "macro note", true)?;
assert_eq!(request.body["model"], "text-embedding-3-small");

// Decode a vector the server would have returned.
let embedding = decode_embedding("text-embedding-3-small", vec![0.1, 0.2, 0.3])?;
assert_eq!(embedding.vector.len(), 3);
# Ok::<(), tdw_embed_openai::OpenAiEmbeddingAdapterError>(())
```

Real client (`--features http`, async):

```rust,ignore
use tdw_embed_openai::OpenAiEmbeddingHttpClient;

let client = OpenAiEmbeddingHttpClient::new(std::env::var("OPENAI_API_KEY")?, "text-embedding-3-small")?;
let embedding = client.embed("macro note").await?;
```

## Example

```text
cargo run --example tdw_embed_openai_basic -p tdw-embed-openai
```

`examples/basic.rs` builds the request body and decodes a fixture vector — no
API key, no network.
