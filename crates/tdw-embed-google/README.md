# tdw-embed-google

Google Gemini Embeddings adapter for the `tdw-embed` `EmbeddingProvider`
contract.

## Purpose

Two layers in one crate:

1. **Offline request builder / decoder** — [`build_embedding_request`] produces
   the JSON body and path for Gemini's `:embedContent` endpoint, and
   [`decode_embedding`] turns a returned `Vec<f32>` into a validated
   `tdw_embed::Embedding`. Both are pure and network-free.
2. **Real HTTP client** (`http` feature) — [`GoogleEmbeddingHttpClient`] posts to
   the Gemini embeddings endpoint with `reqwest`.

Constants: `PROVIDER_ID = "google"`,
`DEFAULT_BASE_URL = "https://generativelanguage.googleapis.com/v1beta"`.

## Feature flags

| Feature | Effect |
| --- | --- |
| `http` | Compiles [`GoogleEmbeddingHttpClient`] and pulls in `reqwest`, `serde`, `tokio`, `async-trait`. **Off by default** so the workspace test set stays offline. |

The request builder / decoder are always available; only the HTTP client is
gated.

## Environment variables

The crate reads no env vars directly. The env-gated **live** integration test
runs only when:

| Variable | Meaning |
| --- | --- |
| `TDW_GOOGLE_EMBEDDING_LIVE=1` | Opt in to the live network test. |
| `TDW_GOOGLE_EMBEDDING_API_KEY`, `GEMINI_API_KEY`, or `GOOGLE_API_KEY` | The API key for the request. |

With these unset the live test early-returns cleanly (no network).

## Quickstart

Offline builder + decoder (no feature, no key):

```rust
use tdw_embed_google::{build_embedding_request, decode_embedding};

let request = build_embedding_request("text-embedding-004", "macro note", true)?;
assert_eq!(request.path, "/models/text-embedding-004:embedContent");
assert_eq!(request.body["model"], "models/text-embedding-004");

let embedding = decode_embedding("text-embedding-004", vec![0.1, 0.2])?;
assert_eq!(embedding.vector.len(), 2);
# Ok::<(), tdw_embed_google::GoogleEmbeddingAdapterError>(())
```

Real client (`--features http`, async):

```rust,ignore
use tdw_embed_google::GoogleEmbeddingHttpClient;

let client = GoogleEmbeddingHttpClient::new(std::env::var("GEMINI_API_KEY")?, "text-embedding-004")?;
let embedding = client.embed("macro note").await?;
```

## Example

```text
cargo run --example tdw_embed_google_basic -p tdw-embed-google
```

`examples/basic.rs` builds the request body and decodes a fixture vector — no
API key, no network.
