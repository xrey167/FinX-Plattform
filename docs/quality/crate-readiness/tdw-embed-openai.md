# tdw-embed-openai Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-embed-openai\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-embed
- External dependencies: serde_json ^1.0.145; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependencies match an offline HTTP request adapter.
- [x] Dependency direction reviewed: depends on the embedding contract only.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: request builder rejects missing API key, empty model, and empty input; decoder rejects empty and non-finite vectors.
- [x] Runtime behavior reviewed: builds JSON request payloads and decodes vectors without live network calls or secret material.
- [x] Tests and coverage evidence recorded: request/decoder test covers credential requirement and invalid payload boundaries.
- [x] Docs and examples reviewed: worksheet records the adapter contract; no separate README/examples required.
- [x] Surface wiring reviewed: no higher-level crate currently depends directly on this adapter.
- [x] Scaffold, dead-code, and fallback signals classified: former stub signal removed; remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: API keys are represented only as caller-supplied presence flags and are never stored.

## Findings

- OpenAI embedding adapter is an offline request/response contract, not a live client.
- Decode path now rejects non-finite vectors in addition to empty vectors.
- Follow-up boundary: HTTP execution, retries, rate limits, and secret loading belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production OpenAI transport/runtime integration.

## Production Backend Evidence (G012)

`OpenAiEmbeddingHttpClient` (gated by `--features http`) at
`crates/tdw-embed-openai/src/http_client.rs` is the real OpenAI
Embeddings HTTP transport. It posts to `POST {base_url}/embeddings`
when the base already ends in `/v1`, and normalizes the default
`https://api.openai.com/v1` base to
`https://api.openai.com/v1/embeddings`. Existing request-builder and
decoder helpers remain available for offline tests and higher-level
dispatchers.

Public surface:
- `OpenAiEmbeddingHttpClient::new(api_key, model_id)` validates the
  bearer token is non-empty and validates the model id through the
  existing adapter error contract.
- `with_base_url(url)` accepts an OpenAI-style `/v1` base or a full
  `/embeddings` endpoint for compatible gateways.
- `model_id()` accessor.
- `async fn embed(input: &str) -> Result<Embedding, OpenAiEmbeddingHttpError>`
  executes the request and decodes the first returned embedding
  vector.

Response shape:
- The first `data[].embedding` vector becomes the workspace
  `tdw_embed::Embedding`.
- Existing `decode_embedding` validation is reused, so empty vectors
  and non-finite values remain typed adapter errors instead of being
  accepted into the embedding contract.
- Empty `data[]` yields `OpenAiEmbeddingHttpError::InvalidResponse`.

Tests:
- Unit tests inside `http_client.rs` cover typed missing-key/model
  errors, debug redaction, endpoint normalization, cassette response
  decoding, empty-data rejection, and empty/non-finite vector
  rejection.
- Integration test at `tests/http_client.rs`, double-gated by
  `--features http` + env var `TDW_OPENAI_EMBEDDING_LIVE=1`. It uses
  `TDW_OPENAI_EMBEDDING_API_KEY` or `OPENAI_API_KEY`, optional
  `TDW_OPENAI_EMBEDDING_BASE_URL`, and optional
  `TDW_OPENAI_EMBEDDING_MODEL` (default `text-embedding-3-small`).

Verification for this slice:
- `cargo +stable test -p tdw-embed-openai --features http --all-targets -- --nocapture`
  passed with the live test skipped because `TDW_OPENAI_EMBEDDING_LIVE`
  was not set.
