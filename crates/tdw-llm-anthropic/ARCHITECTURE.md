# tdw-llm-anthropic — Architecture

## Module map

| File | Contents |
| --- | --- |
| `src/lib.rs` | The offline sync stub [`AnthropicMessagesModel`] (`impl LanguageModel`). Re-exports the `http` module's types when the feature is on. |
| `src/http_client.rs` (`http` feature) | [`AnthropicHttpClient`], the request-body builder, the SSE decoder + stream-state machine, response envelopes, and `parse_response`. |
| `tests/http_client.rs` (`http` feature) | The double-gated live integration test (non-streaming + streaming). Cassette/builder unit tests live *inside* `http_client.rs` because they need `pub(crate)` access. |

## Trait contract

```rust
impl LanguageModel for AnthropicMessagesModel {
    fn model_id(&self) -> &str;
    fn complete(&self, request: ChatRequest) -> tdw_llm::Result<ChatResponse>;
}
```

The stub forwards the last user message (via `tdw_llm::last_user_message`) into a
deterministic `anthropic:<model>:<prompt>` reply. The real `AnthropicHttpClient`
is **not** a `LanguageModel` impl — it exposes its own async `complete` /
`complete_streaming` because the trait is synchronous.

## Request body translation

`build_request_body(model_id, &ChatRequest) -> serde_json::Value`:

- `MessageRole::System` messages are collected into Anthropic's top-level
  `system` field (multiple system messages joined with `\n`).
- `User` / `Assistant` map to `{ "role": …, "content": … }` entries in the
  `messages` array.
- `Tool` is folded into a `user` message with a `[tool] ` prefix — full tool-use
  support is a follow-up.
- `max_output_tokens` → Anthropic's `max_tokens`.

For streaming, the same body gets `"stream": true` set before posting.

## Non-streaming response flow

```
ChatRequest
  └▶ validate_chat_request
  └▶ build_request_body
  └▶ POST /v1/messages  (x-api-key, anthropic-version: 2023-06-01)
        ├─ non-2xx → AnthropicHttpError::Http { status, body }
        └─ 2xx → MessagesEnvelope (serde)
                  └▶ parse_response: concat all `type=="text"` blocks
                       ├─ empty text → InvalidResponse
                       └─ ChatResponse { model_id, message, usage }
```

`parse_response` is extracted so cassette tests can decode canned Anthropic JSON
with no network (see `cassette_replay_decodes_messages_response`).

## SSE streaming flow

`complete_streaming(request, on_delta)`:

```
chunk bytes ─▶ SseDecoder.push() ─▶ Vec<SseEvent>
                 (buffers partial frames; splits on \n\n or \r\n\r\n)
SseEvent ─▶ AnthropicStreamState.apply_event()
   message_start        → seed usage.input/output_tokens
   content_block_delta  → text_delta: call on_delta(text), append to buffer
   message_delta        → update usage (output_tokens; input if >0)
   message_stop         → mark saw_stop
   error                → AnthropicHttpError::InvalidResponse
   ping/​block_start/stop → ignored
   unknown              → ignored (Anthropic versioning policy)
AnthropicStreamState.finish()
   ├─ no text deltas         → InvalidResponse
   ├─ ended before stop      → InvalidResponse
   └─ ChatResponse (trimmed content + accumulated usage)
```

The decoder is robust to chunk boundaries that split a single SSE frame and to
both `\n\n` and `\r\n\r\n` delimiters (`sse_decoder_handles_split_chunks_and_crlf_boundaries`).

## Errors

`AnthropicHttpError`: `MissingApiKey`, `InvalidModelId`, `InvalidBaseUrl`,
`InvalidRequest(LlmError)`, `Http { status, body }`, `Network(reqwest::Error)`,
`InvalidResponse(String)`, `ClientBuild(String)`. The client's `Debug` impl
prints the API key as `REDACTED`.

## Offline cassette-test design

- **Builder / parser / SSE tests** are unit tests in `http_client.rs` that feed
  canned JSON (a "cassette") through `build_request_body`, `parse_response`, and
  the `SseDecoder` + `AnthropicStreamState` — zero network, fully deterministic.
- **Live test** (`tests/http_client.rs`) is compiled only with `--features http`
  and additionally gated on `TDW_ANTHROPIC_LIVE=1` + `ANTHROPIC_API_KEY`; absent
  those it returns early. This keeps `cargo test --workspace` (default features)
  offline.
