# tdw-embed-openai — Architecture

## Module map

| File | Contents |
| --- | --- |
| `src/lib.rs` | `EmbeddingHttpRequest` DTO, `OpenAiEmbeddingAdapterError`, the pure `build_embedding_request` / `decode_embedding` functions, and the `PROVIDER_ID` / `DEFAULT_BASE_URL` / `EMBEDDINGS_PATH` constants. Re-exports the `http` types when the feature is on. |
| `src/http_client.rs` (`http` feature) | [`OpenAiEmbeddingHttpClient`] (`impl EmbeddingProvider`), URL normalization, the `EmbeddingsEnvelope` decoder, and `parse_response`. |
| `tests/http_client.rs` (`http` feature) | The double-gated live integration test. Cassette/builder unit tests live inside `http_client.rs`. |

## Contract

The crate plugs into `tdw_embed::EmbeddingProvider` via the HTTP client:

```rust
#[async_trait::async_trait]
impl EmbeddingProvider for OpenAiEmbeddingHttpClient {
    fn model_id(&self) -> &str;
    async fn embed(&self, text: &str) -> tdw_embed::Result<Embedding>;
}
```

The crate-specific `OpenAiEmbeddingHttpError` is flattened onto
`EmbeddingError::Provider(String)` when driven through the trait, preserving the
message.

## Request builder

`build_embedding_request(model, input, api_key_present) -> EmbeddingHttpRequest`:

- Requires `api_key_present == true` (else `MissingApiKey`) — the function never
  reads or stores a key, it only asserts the caller has one.
- Trims + rejects empty `model` (`EmptyModel`) and `input` (`EmptyInput`).
- Produces `body = { "model": model, "input": input }`, `path = "/embeddings"`,
  `bearer_token_required = true`.

## Vector decoder

`decode_embedding(model, vector) -> Embedding`:

- Rejects empty `model`, empty `vector` (`EmptyVector`), and any non-finite
  component (`NonFiniteVector`).
- Returns `Embedding { model_id, vector }`.

## Response flow (`http` feature)

```
input ─▶ build_embedding_request ─▶ POST {base}/v1/embeddings (bearer)
       ├─ non-2xx → Http { status, body }
       └─ 2xx → EmbeddingsEnvelope (serde)
                 └▶ parse_response: take data[0].embedding
                      └▶ decode_embedding (validate) → Embedding
```

`embeddings_url` normalizes a `/v1` base or a full `/embeddings` endpoint to the
final URL. `Debug` redacts the API key.

## Offline cassette-test design

`build_embedding_request` / `decode_embedding` are unit-tested directly with no
network (`builds_request_and_decodes_vector_without_network_call`). The response
decoder is exercised by cassette tests in `http_client.rs` that feed canned
OpenAI embeddings JSON through `parse_response`. The live test is compiled only
with `--features http` and gated on `TDW_OPENAI_EMBEDDING_LIVE=1` + an API key.
