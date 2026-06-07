# tdw-llm-openai-compat — Architecture

## Module map

| File | Contents |
| --- | --- |
| `src/lib.rs` | The offline sync stub [`OpenAiCompatibleModel`] (`impl LanguageModel`), which carries an optional validated `base_url`. Re-exports the `http` types when the feature is on. |
| `src/http_client.rs` (`http` feature) | [`OpenAiCompatibleHttpClient`], the request-body builder, the SSE decoder + stream-state machine, response envelopes, and `parse_response`. |
| `tests/http_client.rs` (`http` feature) | The double-gated live integration test. Cassette/builder unit tests live inside `http_client.rs` (need `pub(crate)`). |

## Trait contract

```rust
impl LanguageModel for OpenAiCompatibleModel {
    fn model_id(&self) -> &str;
    fn complete(&self, request: ChatRequest) -> tdw_llm::Result<ChatResponse>;
}
```

`OpenAiCompatibleModel::new(model_id, base_url)` validates the model id and, when
present, the base URL (`validate_base_url`). The real `OpenAiCompatibleHttpClient`
is async and separate (the trait is synchronous).

## Auth modes

- `OpenAiCompatibleHttpClient::new(api_key, model_id)` — authenticated; sends
  `Authorization: Bearer <key>`.
- `OpenAiCompatibleHttpClient::without_api_key(model_id)` — for local/private
  gateways (Ollama, vLLM) that need no bearer token.

## Endpoint normalization

`chat_completions_url` normalizes any supplied base URL to end in
`/v1/chat/completions`:

| Input base URL | Resolved endpoint |
| --- | --- |
| `http://localhost:11434` | `…/v1/chat/completions` |
| `http://localhost:11434/v1` | `…/v1/chat/completions` |
| `https://api.openai.com` (default) | `…/v1/chat/completions` |

## Request body translation

`build_request_body` maps `System`/`User`/`Assistant` to their OpenAI roles and
folds `Tool` into a `user` message prefixed with `[tool] `. `max_output_tokens`
→ `max_tokens`. Streaming additionally sets `stream: true` and
`stream_options.include_usage: true`.

## Non-streaming response flow

```
ChatRequest ─▶ validate_chat_request ─▶ build_request_body
            ─▶ POST /v1/chat/completions (optional bearer)
                ├─ non-2xx → Http { status, body }
                └─ 2xx → ChatCompletionEnvelope (serde)
                          └▶ parse_response: first non-empty choice content
                               ├─ none → InvalidResponse
                               └─ ChatResponse { model_id, message, usage }
```

Usage decoding accepts both OpenAI (`prompt_tokens`/`completion_tokens`) and
compat aliases (`input_tokens`/`output_tokens`) via serde `alias`.

## SSE streaming flow

```
chunk bytes ─▶ SseDecoder.push() ─▶ Vec<SseEvent>  (data-only frames)
SseEvent ─▶ OpenAiStreamState.apply_event()
   "[DONE]"          → mark saw_done
   error object      → InvalidResponse
   usage chunk       → set input/output tokens
   choices[].delta.content → on_delta(content), append to buffer
   choices[].finish_reason → mark saw_finish_reason
OpenAiStreamState.finish()
   ├─ no text deltas               → InvalidResponse
   ├─ no [DONE] and no finish_reason → InvalidResponse
   └─ ChatResponse (trimmed content + usage)
```

The decoder handles chunk splits mid-frame and both `\n\n` / `\r\n\r\n`
delimiters.

## Errors

`OpenAiCompatibleHttpError`: `MissingApiKey`, `InvalidModelId`, `InvalidBaseUrl`,
`InvalidRequest(LlmError)`, `Http { status, body }`, `Network(reqwest::Error)`,
`InvalidResponse(String)`, `ClientBuild(String)`. `Debug` redacts the key.

## Offline cassette-test design

Builder / parser / SSE behaviour is covered by unit tests in `http_client.rs`
that feed canned Chat Completions JSON (a "cassette") through the pure functions
— no network. The live test (`tests/http_client.rs`) is compiled only with
`--features http` and gated on `TDW_OPENAI_COMPAT_LIVE=1` + an API key, so the
default workspace test run is offline.
