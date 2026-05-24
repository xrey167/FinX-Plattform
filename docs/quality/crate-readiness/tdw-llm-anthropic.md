# tdw-llm-anthropic Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-llm-anthropic\Cargo.toml
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
- [x] Public API and error contracts reviewed: constructor validates model ID and completion uses shared chat-request validation.
- [x] Runtime behavior reviewed: adapter is a deterministic offline model stub and performs no live Anthropic network call.
- [x] Tests and coverage evidence recorded: test covers valid completion, invalid model ID, control-character model IDs, and invalid prompt content.
- [x] Docs and examples reviewed: no separate README/examples required for this offline adapter contract.
- [x] Surface wiring reviewed: service-api imports the Anthropic adapter.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: invalid prompts and model identifiers fail through the shared contract.

## Findings

- Anthropic adapter remains deterministic and transport-free while preserving provider-specific model output labeling.
- New validation evidence proves it does not bypass the shared chat/model checks.
- Follow-up boundary: real Anthropic Messages API HTTP transport, credentials, and streaming belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production Anthropic transport/runtime integration.

## Production Backend Evidence (G012)

`AnthropicHttpClient` (gated by `--features http`) at
`crates/tdw-llm-anthropic/src/http_client.rs` is the first real LLM
HTTP transport in the workspace. Posts to
`POST {base_url}/v1/messages` with the standard Anthropic Messages
API headers (`x-api-key`, `anthropic-version: 2023-06-01`).
Existing sync stub `AnthropicMessagesModel` preserved as offline
default.

Public surface:
- `AnthropicHttpClient::new(api_key, model_id)` — validates the
  model id via `tdw_llm::validate_model_id`.
- `with_base_url(url)` — override default
  `https://api.anthropic.com` for tests / self-hosted gateways.
- `model_id()` accessor.
- `async fn complete(request: ChatRequest) -> Result<ChatResponse, AnthropicHttpError>`
  — the primary surface. Async-native because `tdw_llm::LanguageModel`
  is a synchronous trait; bridging sync/async is a follow-up
  concern.

Role translation:
- `MessageRole::System` messages collapse into Anthropic's
  top-level `system` field (joined with `\n` if multiple).
- `MessageRole::User` / `Assistant` go into the `messages` array
  with the matching role string.
- `MessageRole::Tool` is folded into a `user` message with a
  `[tool] ` prefix for this slice; full tool-use envelope support
  is a follow-up.

Response shape:
- All `content[]` blocks with `"type": "text"` are concatenated
  into `ChatResponse.message.content`. Empty text content yields
  `AnthropicHttpError::InvalidResponse`.
- `usage.input_tokens` / `usage.output_tokens` flow through to
  `ChatResponse.usage`.

Streaming (`stream: true` SSE) is deferred to a follow-up slice;
this PR ships the batch endpoint only.

Tests:
- Unit tests inside `http_client.rs` (need `pub(crate)` access to
  `build_request_body` + `parse_response` + the envelope types):
  - `system_message_becomes_top_level_system_field_and_user_goes_in_messages`
  - `multiple_system_messages_join_with_newline`
  - `tool_role_is_folded_into_user_message_with_marker`
  - `cassette_replay_decodes_messages_response`
  - `cassette_replay_joins_multiple_text_blocks`
  - `cassette_replay_errors_when_no_text_content`
- Integration test at `tests/http_client.rs`, double-gated by
  `--features http` + env vars `TDW_ANTHROPIC_LIVE=1` and
  `ANTHROPIC_API_KEY`. Calls the cheapest model
  (`claude-haiku-4-5-20251001`) with a one-message prompt; asserts
  non-empty content + non-zero `output_tokens`. Costs ~$0.001 per
  run.

`tdw_core::Credentials` gained an `anthropic_api_key: Option<String>`
field as part of this slice so deployments can supply the key
through the standard credentials surface.
