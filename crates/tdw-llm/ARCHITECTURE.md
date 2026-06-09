# tdw-llm — Architecture

## Module map

Single-file crate (`src/lib.rs`). It is `#![forbid(unsafe_code)]` and contains:

| Item | Role |
| --- | --- |
| `LlmError` / `Result<T>` | The crate error enum and result alias. Validation-only variants (`EmptyMessages`, `EmptyMessageContent`, `EmptyMaxOutputTokens`, `EmptyModelId`, `InvalidModelId`, `InvalidBaseUrl`, `UnsafeBaseUrl`, `UnsupportedProvider`). |
| `MessageRole` | `System` / `User` / `Assistant` / `Tool`. |
| `ChatMessage` | `{ role, content }`. |
| `ChatRequest` | `{ messages, max_output_tokens }`. |
| `Usage` | `{ input_tokens, output_tokens }`. |
| `ChatResponse` | `{ model_id, message, usage }`. |
| `LanguageModel` | The trait. |
| `ModelSelection` | `{ provider, model, base_url }` + `from_config`. |
| free fns | `last_user_message`, `validate_chat_request`, `validate_model_id`, `validate_base_url`. |

## Trait contract: `LanguageModel`

```rust
pub trait LanguageModel: Send + Sync {
    fn model_id(&self) -> &str;
    fn complete(&self, request: ChatRequest) -> Result<ChatResponse>;
}
```

- **Synchronous by design.** The workspace's canonical contract is `fn complete`
  (not `async fn`). Provider crates that need a real async HTTP client (e.g.
  `tdw-llm-anthropic`'s `http` module) expose an *additional* async client type
  rather than changing this trait; bridging async-over-sync is left to the
  caller.
- `Send + Sync` so a model can live behind an `Arc<dyn LanguageModel>` and be
  shared across threads (see `tdw-eval-runner::EvalRunner`).
- Implementations should validate their inputs through the shared validators so
  every adapter rejects the same malformed requests identically.

## Validation contract

`complete` implementations call into the shared validators before doing work:

- `validate_chat_request` — non-empty `messages`, `max_output_tokens > 0`, and no
  message whose trimmed `content` is empty.
- `validate_model_id` — non-empty after trim, no control characters.
- `validate_base_url` — must start with `http://`/`https://`, and contain no
  whitespace or control characters (defends against header/URL injection).
- `last_user_message` — validates then returns the content of the **last**
  `User` message; the prompt most adapters forward.

Centralising these here means a new provider adapter gets identical
request-hygiene guarantees "for free".

## Request flow

```
caller ─▶ ChatRequest
            │  validate_chat_request / last_user_message
            ▼
   impl LanguageModel::complete  (per-provider adapter)
            │  build provider body → (HTTP, in adapter's `http` module)
            ▼
        ChatResponse { model_id, message, usage }
```

Streaming (SSE) is **not** part of this trait — it is an adapter concern. See
`tdw-llm-anthropic`'s `ARCHITECTURE.md` for the SSE decoder + stream-state design
that turns `text_delta` events into an accumulated `ChatResponse`.

## Offline / cassette-test design

The crate's own tests use an in-module `EchoModel` and assert validator
behaviour. There is no network, so the default `cargo test` run is offline and
deterministic. Provider adapters follow the same principle: the cassette
(fixture) tests that decode canned provider JSON live next to the decoder as
unit tests, while live tests are double-gated behind a feature **and** a
`TDW_*_LIVE` env var.
