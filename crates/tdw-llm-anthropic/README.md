# tdw-llm-anthropic

Anthropic Messages adapter for the `tdw-llm` `LanguageModel` contract.

## Purpose

Two layers in one crate:

1. **Offline sync stub** — [`AnthropicMessagesModel`] implements
   [`tdw_llm::LanguageModel`] synchronously. It validates the model id and the
   request, then returns a deterministic, network-free `ChatResponse`. This is
   what tests and offline tooling use.
2. **Real HTTP client** (`http` feature) — [`AnthropicHttpClient`] posts to
   `POST /v1/messages` with `reqwest`, supports non-streaming
   ([`AnthropicHttpClient::complete`]) and SSE streaming
   ([`AnthropicHttpClient::complete_streaming`]), and translates the response
   back into a workspace `ChatResponse`. It is **async-native** because the
   `LanguageModel` trait is synchronous and bridging async-over-sync is a
   separate concern.

## Feature flags

| Feature | Effect |
| --- | --- |
| `http` | Compiles [`AnthropicHttpClient`] and pulls in `reqwest`, `serde`, `serde_json`, `thiserror`, `tokio`. **Off by default** so the workspace test set stays offline. |

The sync stub is always available; only the HTTP client is gated.

## Environment variables

The crate reads no env vars directly. The env-gated **live** integration test
(`tests/http_client.rs`, compiled only with `--features http`) runs only when
both are set:

| Variable | Meaning |
| --- | --- |
| `TDW_ANTHROPIC_LIVE=1` | Opt in to the live network test. |
| `ANTHROPIC_API_KEY` | The API key passed to `AnthropicHttpClient::new`. |

With either unset the live test early-returns cleanly (no network).

## Quickstart

Offline stub (no feature, no key):

```rust
use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};
use tdw_llm_anthropic::AnthropicMessagesModel;

let model = AnthropicMessagesModel::new("claude-haiku-4-5-20251001")?;
let response = model.complete(ChatRequest {
    messages: vec![ChatMessage { role: MessageRole::User, content: "Say hi.".into() }],
    max_output_tokens: 32,
})?;
# Ok::<(), tdw_llm::LlmError>(())
```

Real client (`--features http`, async):

```rust,ignore
use tdw_llm::{ChatMessage, ChatRequest, MessageRole};
use tdw_llm_anthropic::AnthropicHttpClient;

let client = AnthropicHttpClient::new(std::env::var("ANTHROPIC_API_KEY")?, "claude-haiku-4-5-20251001")?;
let response = client.complete(ChatRequest {
    messages: vec![ChatMessage { role: MessageRole::User, content: "Say hi.".into() }],
    max_output_tokens: 32,
}).await?;
```

`AnthropicHttpClient::with_base_url` overrides the endpoint for self-hosted
Anthropic-compatible gateways. The client's `Debug` impl redacts the API key.

## Example

```text
cargo run --example tdw_llm_anthropic_basic -p tdw-llm-anthropic
```

`examples/basic.rs` builds a request and completes it through the offline sync
stub — no API key, no network.
