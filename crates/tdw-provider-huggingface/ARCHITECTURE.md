# Architecture — tdw-provider-huggingface

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | `InferenceRequest` contract type, `HuggingFaceTextGenerationQuery` (with `with_max_new_tokens`), `HuggingFaceTextGeneration`, `HuggingFaceProviderError`, model-id validation, and the `text_generation_request` request-contract builder. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `HuggingFaceHttpTextGenerationFetcher`, the token-env lookup, and private serde shapes for generations / error envelopes. |

Public constants: `PROVIDER_ID = "huggingface"`, `BASE_URL`,
`AUTH_HEADER = "Authorization"`. The token env names
(`HF_TOKEN`, `HUGGINGFACE_API_TOKEN`, `HF_API_TOKEN`) are declared privately in
`http_fetcher.rs`.

## Traits

`HuggingFaceHttpTextGenerationFetcher` implements
`tdw_core::Fetcher<HuggingFaceTextGenerationQuery, HuggingFaceTextGeneration>`:

- `const PROVIDER = "huggingface"`, `const ENDPOINT = "text_generation"`.
- `transform_query` accepts `model_id`/`model` and `inputs`/`prompt`, plus an
  optional `max_new_tokens`, re-validating through the typed constructor.
- `extract_data` re-checks the request contract, resolves the bearer token from
  the env chain, and `POST`s `{ "inputs", "parameters": { "max_new_tokens" } }`.
- `transform_data` tolerates three response shapes: an `{ "error" }` envelope
  (surfaced as a `Provider` error), an array of generations, and a single
  generation object.

`registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No
`provider_fetcher_struct!` macro; the struct is hand-written for `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ HuggingFaceTextGenerationQuery
                                     │
                                     ▼ extract_data  (HTTP POST, feature = "http")
                              raw Bytes (JSON)
                                     │
                                     ▼ transform_data (3 shapes)
                              Vec<HuggingFaceTextGeneration>
```

This is the only non-`GET` provider in the batch; the request body carries the
prompt and parameters.

## Offline / cassette design

`transform_data` is pure, so cassette tests feed recorded generation `Bytes`
with no network. The `text_generation_request` builder and model-id validation
are unit-tested without the `http` feature; `examples/basic.rs` drives
`transform_query` + `transform_data` over an inline generation fixture. The live
test is gated by `TDW_HUGGINGFACE_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The serde shapes mirror only the public HF Inference wire format; no vendor SDK
  is vendored.
- Network access lives solely behind the `http` feature.
- Model-id validation rejects `/`-leading/trailing, `//`, and `.`/`..` segments
  to prevent path traversal; the bearer token is resolved at request time and
  never logged. Errors map into `tdw_core::Error::{InvalidQuery, Provider}`.
