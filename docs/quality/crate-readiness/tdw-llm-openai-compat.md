# tdw-llm-openai-compat Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-llm-openai-compat\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-llm
- External dependencies: none
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependency shape match a model adapter.
- [x] Dependency direction reviewed: depends only on tdw-llm; service-api consumes it.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: constructor validates model IDs and optional base URLs; completion uses shared chat-request validation.
- [x] Runtime behavior reviewed: adapter is deterministic and offline while preserving OpenAI-compatible base URL configuration.
- [x] Tests and coverage evidence recorded: test covers valid base URL, invalid model/base URLs, and invalid prompt content.
- [x] Docs and examples reviewed: no separate README/examples required for this offline adapter contract.
- [x] Surface wiring reviewed: service-api imports the OpenAI-compatible adapter.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: unsafe base URLs and malformed prompts fail before any future transport layer.

## Findings

- OpenAI-compatible adapter keeps base URL configuration explicit and validated.
- Completion remains a deterministic offline contract with no live network dependency.
- Follow-up boundary: actual OpenAI-compatible HTTP transport, credentials, streaming, and tool-calling belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production OpenAI-compatible transport/runtime integration.

## Production Backend Evidence (G012)

`OpenAiCompatibleHttpClient` (gated by `--features http`) at
`crates/tdw-llm-openai-compat/src/http_client.rs` is the real
OpenAI-compatible Chat Completions transport. It posts to
`POST {base_url}/v1/chat/completions` and defaults to
`https://api.openai.com`. Existing sync stub
`OpenAiCompatibleModel` remains the offline default.

Public surface:
- `OpenAiCompatibleHttpClient::new(api_key, model_id)` validates
  the bearer token is non-empty and validates the model id via
  `tdw_llm::validate_model_id`.
- `OpenAiCompatibleHttpClient::without_api_key(model_id)` supports
  local/private compatible gateways that intentionally do not
  require bearer auth.
- `with_base_url(url)` accepts a host root such as
  `http://localhost:11434` or an existing `/v1` base and normalizes
  the completion endpoint to `/v1/chat/completions`.
- `model_id()` accessor.
- `async fn complete(request: ChatRequest) -> Result<ChatResponse, OpenAiCompatibleHttpError>`
  is the primary surface. Async-native because `tdw_llm::LanguageModel`
  is a synchronous trait; bridging sync/async is a follow-up concern.

Role translation:
- `MessageRole::System`, `User`, and `Assistant` map directly to the
  same OpenAI Chat Completions role strings.
- `MessageRole::Tool` is folded into a `user` message with a
  `[tool] ` prefix for this slice; full tool-call envelope support is
  a follow-up.

Response shape:
- The first non-empty `choices[].message.content` becomes
  `ChatResponse.message.content`. Empty/missing assistant text yields
  `OpenAiCompatibleHttpError::InvalidResponse`.
- `usage.prompt_tokens` / `usage.completion_tokens` flow through to
  `ChatResponse.usage`; compatible aliases `input_tokens` /
  `output_tokens` are accepted for non-OpenAI gateways.

Streaming (`stream: true` SSE) is deferred to a follow-up slice; this
PR ships the batch endpoint only.

Tests:
- Unit tests inside `http_client.rs` cover typed missing-key errors,
  debug redaction, base URL normalization, role/body translation,
  cassette response decoding, compatible usage aliases, and malformed
  cassette rejection.
- Integration test at `tests/http_client.rs`, double-gated by
  `--features http` + env var `TDW_OPENAI_COMPAT_LIVE=1`. It uses
  `TDW_OPENAI_COMPAT_API_KEY` or `OPENAI_API_KEY`, optional
  `TDW_OPENAI_COMPAT_BASE_URL`, and optional
  `TDW_OPENAI_COMPAT_MODEL` (default `gpt-4o-mini`).

Verification for this slice:
- `cargo +stable test -p tdw-llm-openai-compat --features http --all-targets -- --nocapture`
  passed with the live test skipped because `TDW_OPENAI_COMPAT_LIVE`
  was not set.
