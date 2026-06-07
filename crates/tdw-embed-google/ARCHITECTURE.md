# tdw-embed-google — Architecture

## Module map

| File | Contents |
| --- | --- |
| `src/lib.rs` | `EmbeddingHttpRequest` DTO, `GoogleEmbeddingAdapterError`, the pure `build_embedding_request` / `decode_embedding` functions, and the `PROVIDER_ID` / `DEFAULT_BASE_URL` constants. Re-exports the `http` types when the feature is on. |
| `src/http_client.rs` (`http` feature) | [`GoogleEmbeddingHttpClient`], the response decoder, and `parse_response`. |
| `tests/http_client.rs` (`http` feature) | The double-gated live integration test. Cassette/builder unit tests live in `src`. |

## Contract

The HTTP client implements `tdw_embed::EmbeddingProvider` (`async fn embed`), so
it is a drop-in for `tdw-knowledge` behind `Arc<dyn EmbeddingProvider>`.

## Request builder

`build_embedding_request(model, input, api_key_present) -> EmbeddingHttpRequest`:

- Requires `api_key_present == true` (else `MissingApiKey`); never reads a key.
- Trims + rejects empty `model` (`EmptyModel`) and `input` (`EmptyInput`).
- Produces the Gemini shape, which differs from OpenAI:
  - `path = "/models/{model}:embedContent"`,
  - `body = { "model": "models/{model}", "content": { "parts": [{ "text": input }] } }`,
  - `api_key_required = true`.

## Vector decoder

`decode_embedding(model, vector) -> Embedding` — same hygiene as the OpenAI
adapter: rejects empty model, empty vector (`EmptyVector`), non-finite
components (`NonFiniteVector`).

## Response flow (`http` feature)

```
input ─▶ build_embedding_request ─▶ POST {base}{path}?key=…
       ├─ non-2xx → Http error
       └─ 2xx → envelope (serde)
                 └▶ parse_response → decode_embedding (validate) → Embedding
```

## Offline cassette-test design

`build_embedding_request` / `decode_embedding` are unit-tested with no network
(`builds_request_and_decodes_vector_without_network_call`, which also asserts the
Gemini `parts[].text` body shape). The live test is compiled only with
`--features http` and gated on `TDW_GOOGLE_EMBEDDING_LIVE=1` + an API key, so the
default workspace test run is offline.
