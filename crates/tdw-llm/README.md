# tdw-llm

Provider-agnostic language-model contract for the TDW workspace.

## Purpose

`tdw-llm` defines the small, synchronous trait and DTOs every LLM provider
adapter (`tdw-llm-anthropic`, `tdw-llm-openai-compat`, …) implements, plus the
request/model-id/base-url validators they share. It owns **no** network code and
pulls in **no** HTTP dependency, so it compiles and tests fully offline.

Core surface:

- [`LanguageModel`] — `fn model_id(&self) -> &str` and
  `fn complete(&self, ChatRequest) -> Result<ChatResponse>`.
- [`ChatRequest`] / [`ChatMessage`] / [`MessageRole`] / [`ChatResponse`] /
  [`Usage`] — the on-wire-agnostic chat DTOs.
- [`ModelSelection`] — provider/model/base-url triple, buildable from
  `TdwConfig` via [`ModelSelection::from_config`].
- Validators: [`validate_chat_request`], [`validate_model_id`],
  [`validate_base_url`], and the [`last_user_message`] helper.

## Feature flags

None. This crate is intentionally dependency-light (`serde`, `tdw-config`,
`thiserror`) and has no optional features. The `http` feature lives on the
provider adapter crates that depend on this one.

## Environment variables

None are read by this crate. API keys and the `TDW_*_LIVE` gates are consumed by
the provider adapters (e.g. `ANTHROPIC_API_KEY` + `TDW_ANTHROPIC_LIVE` in
`tdw-llm-anthropic`).

## Quickstart

```rust
use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};

fn ask(model: &dyn LanguageModel) -> tdw_llm::Result<String> {
    let response = model.complete(ChatRequest {
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: "Summarize AAPL".to_string(),
        }],
        max_output_tokens: 256,
    })?;
    Ok(response.message.content)
}
```

See `examples/basic.rs` for a self-contained, offline example that builds a
request and completes it through a tiny in-crate `LanguageModel` implementation:

```text
cargo run --example tdw_llm_basic -p tdw-llm
```

## Related crates

- `tdw-llm-anthropic` — Anthropic Messages adapter (sync stub + `http` client).
- `tdw-llm-openai-compat` — OpenAI-compatible Chat Completions adapter.
- `tdw-eval-runner` — drives a `LanguageModel` over an eval dataset.
