# tdw-embed-google Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-embed-google\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-embed
- External dependencies: reqwest ^0.12.24 (optional); serde ^1.0.228 (optional); serde_json ^1.0.150; thiserror ^2.0.18; tokio ^1.52.3 (optional)
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: default=[]; http
- Test attributes detected: 7
- tests/ directory: yes
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, default-offline dependencies, and optional HTTP execution dependencies.
- [x] Dependency direction reviewed: depends on the embedding contract only.
- [x] Feature flags reviewed: `http` gates reqwest, serde, and tokio so default workspace tests stay offline.
- [x] Public API and error contracts reviewed: request builder rejects missing API key, empty model, and empty input; decoder rejects empty and non-finite vectors.
- [x] Runtime behavior reviewed: default path builds Google embedContent request payloads without live network calls; `http` path executes real Gemini requests via reqwest.
- [x] Tests and coverage evidence recorded: request/decoder tests cover credential requirement and invalid payload boundaries; HTTP cassette tests cover response parsing, endpoint normalization, and typed errors.
- [x] Docs and examples reviewed: worksheet records the adapter contract; no separate README/examples required.
- [x] Surface wiring reviewed: no higher-level crate currently depends directly on this adapter.
- [x] Scaffold, dead-code, and fallback signals classified: former stub signal removed; remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: HTTP client uses `x-goog-api-key`; debug output redacts stored API keys; live tests are explicitly env-gated.

## Findings

- Google embedding adapter now has a real Gemini HTTP client behind the `http` feature while keeping the default crate offline.
- Decode path now rejects non-finite vectors in addition to empty vectors.
- Follow-up boundary: retries, rate limits, and secret loading belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains, and the G012 Google embedding production transport has landed behind an opt-in feature flag.

## Production Backend Evidence (G012)

`GoogleEmbeddingHttpClient` (gated by `--features http`) at
`crates/tdw-embed-google/src/http_client.rs` is the real Gemini
Embeddings HTTP transport. It posts to
`POST {base_url}/models/{model}:embedContent` when the base already
ends in `/v1beta`, and normalizes the default
`https://generativelanguage.googleapis.com/v1beta` base to
`https://generativelanguage.googleapis.com/v1beta/models/{model}:embedContent`.
Existing request-builder and decoder helpers remain available for
offline tests and higher-level dispatchers.

Public surface:
- `GoogleEmbeddingHttpClient::new(api_key, model_id)` validates the
  API key is non-empty, accepts either bare model ids or
  `models/{model}` resource names, and validates the model id through
  the existing adapter error contract.
- `with_base_url(url)` accepts the Gemini API root, a versioned
  `/v1beta` base, or a full `:embedContent` endpoint for compatible
  gateways.
- `model_id()` accessor.
- `async fn embed(input: &str) -> Result<Embedding, GoogleEmbeddingHttpError>`
  executes the request with `x-goog-api-key` authentication and
  decodes the returned embedding vector.

Response shape:
- `embedding.values[]` becomes the workspace `tdw_embed::Embedding`.
- Existing `decode_embedding` validation is reused, so empty vectors
  and non-finite values remain typed adapter errors instead of being
  accepted into the embedding contract.
- Missing `embedding` yields
  `GoogleEmbeddingHttpError::InvalidResponse`.

Tests:
- Unit tests inside `http_client.rs` cover typed missing-key/model
  errors, debug redaction, endpoint normalization, cassette response
  decoding, missing-embedding rejection, and empty/non-finite vector
  rejection.
- Integration test at `tests/http_client.rs`, double-gated by
  `--features http` + env var `TDW_GOOGLE_EMBEDDING_LIVE=1`. It uses
  `TDW_GOOGLE_EMBEDDING_API_KEY`, `GEMINI_API_KEY`, or
  `GOOGLE_API_KEY`, optional `TDW_GOOGLE_EMBEDDING_BASE_URL`, and
  optional `TDW_GOOGLE_EMBEDDING_MODEL` (default
  `gemini-embedding-001`).

Verification for this slice:
- `cargo +stable test -p tdw-embed-google --features http --all-targets -- --nocapture`
  passed with the live test skipped because
  `TDW_GOOGLE_EMBEDDING_LIVE` was not set.
- `cargo +stable test -p tdw-embed-google --all-targets -- --nocapture`
  passed.
- `cargo +stable clippy -p tdw-embed-google --features http --all-targets -- -D warnings`
  passed.
- `cargo +stable clippy -p tdw-embed-google --all-targets -- -D warnings`
  passed.
- `cargo +stable clippy --workspace --all-targets -- -D warnings`
  passed.
- `cargo +stable test --workspace` passed.
- `cargo +stable run -p xtask -- clean-room-audit` passed.
- `cargo +stable fmt --all -- --check` and `git diff --check`
  passed.
