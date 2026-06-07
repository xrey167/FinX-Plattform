# tdw-llm-openai-compat

OpenAI-compatible Chat Completions adapter for the `tdw-llm` `LanguageModel`
contract. Works against the OpenAI API and any compatible gateway (Ollama,
vLLM, LiteLLM, …) via a custom base URL.

## Purpose

Two layers in one crate:

1. **Offline sync stub** — [`OpenAiCompatibleModel`] implements
   [`tdw_llm::LanguageModel`] synchronously, validating the model id and (when
   present) the base URL, then returning a deterministic, network-free response.
2. **Real HTTP client** (`http` feature) — [`OpenAiCompatibleHttpClient`] posts
   to the Chat Completions endpoint with `reqwest`.

## Feature flags

| Feature | Effect |
| --- | --- |
| `http` | Compiles [`OpenAiCompatibleHttpClient`] and pulls in `reqwest`, `serde`, `serde_json`, `thiserror`, `tokio`. **Off by default** so the workspace test set stays offline. |

The sync stub is always available; only the HTTP client is gated.

## Environment variables

The crate reads no env vars directly. The env-gated **live** integration test
(`tests/http_client.rs`, compiled only with `--features http`) runs only when:

| Variable | Meaning |
| --- | --- |
| `TDW_OPENAI_COMPAT_LIVE=1` | Opt in to the live network test. |
| `OPENAI_API_KEY` or `TDW_OPENAI_COMPAT_API_KEY` | The API key for the request. |

With these unset the live test early-returns cleanly (no network).

## Quickstart

Offline stub (no feature, no key):

```rust
use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};
use tdw_llm_openai_compat::OpenAiCompatibleModel;

// Optional base_url targets a self-hosted gateway (e.g. Ollama).
let model = OpenAiCompatibleModel::new("gpt-compatible", Some("http://localhost:11434".into()))?;
let response = model.complete(ChatRequest {
    messages: vec![ChatMessage { role: MessageRole::User, content: "draft".into() }],
    max_output_tokens: 64,
})?;
assert_eq!(model.base_url(), Some("http://localhost:11434"));
# Ok::<(), tdw_llm::LlmError>(())
```

The base URL is validated by `tdw_llm::validate_base_url`: it must start with
`http://`/`https://` and contain no whitespace or control characters.

## Example

```text
cargo run --example tdw_llm_openai_compat_basic -p tdw-llm-openai-compat
```

`examples/basic.rs` builds a request and completes it through the offline sync
stub — no API key, no network.
