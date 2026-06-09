//! Real Anthropic Messages API client for G012.
//!
//! Gated by the `http` feature. Posts to `POST /v1/messages` against
//! Anthropic's public API (or any compatible base URL via
//! [`AnthropicHttpClient::with_base_url`]). The existing sync stub
//! [`crate::AnthropicMessagesModel`] is preserved as-is for offline
//! tests; this client is **async-native** because the workspace's
//! `LanguageModel` trait is synchronous (`fn complete(&self, …)`)
//! and bridging async-over-sync is a separate concern.

use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_llm::{
    ChatMessage, ChatRequest, ChatResponse, ClassifyError, MessageRole, Retryability, Usage,
    classify_http_status, validate_chat_request, validate_model_id,
};
use thiserror::Error;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Production Anthropic Messages API client.
#[derive(Clone)]
pub struct AnthropicHttpClient {
    client: Client,
    base_url: Url,
    api_key: String,
    model_id: String,
}

impl std::fmt::Debug for AnthropicHttpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AnthropicHttpClient")
            .field("base_url", &self.base_url.as_str())
            .field("model_id", &self.model_id)
            .field("api_key", &"REDACTED")
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum AnthropicHttpError {
    #[error("anthropic api key must not be empty")]
    MissingApiKey,
    #[error("invalid model id: {0}")]
    InvalidModelId(String),
    #[error("invalid base url: {0}")]
    InvalidBaseUrl(String),
    #[error("chat request invalid: {0}")]
    InvalidRequest(tdw_llm::LlmError),
    #[error("anthropic http {status}: {body}")]
    Http {
        status: reqwest::StatusCode,
        body: String,
    },
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("response shape error: {0}")]
    InvalidResponse(String),
    #[error("client build error: {0}")]
    ClientBuild(String),
}

impl ClassifyError for AnthropicHttpError {
    /// Translate the transport-facing variants into a recoverability
    /// classification using the shared `tdw-llm` helpers; all other
    /// (local validation) variants are permanent.
    ///
    /// `Network(_)` (a reqwest transport failure) is treated as transient.
    /// `Http { status, body }` is delegated to
    /// [`tdw_llm::classify_http_status`], which owns the transient status
    /// set and the context-overflow heuristic.
    fn retryability(&self) -> Retryability {
        match self {
            Self::Network(_) => Retryability::Transient,
            Self::Http { status, body } => classify_http_status(status.as_u16(), body),
            _ => Retryability::Permanent,
        }
    }
}

impl AnthropicHttpClient {
    /// Construct a client for a specific Anthropic model id (e.g.
    /// `"claude-haiku-4-5-20251001"`). Defaults to the public API
    /// base URL.
    pub fn new(
        api_key: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, AnthropicHttpError> {
        let api_key = normalize_api_key(api_key.into())?;
        let model_id = model_id.into();
        validate_model_id(&model_id)
            .map_err(|_| AnthropicHttpError::InvalidModelId(model_id.clone()))?;
        let base_url = Url::parse(DEFAULT_BASE_URL)
            .map_err(|error| AnthropicHttpError::InvalidBaseUrl(error.to_string()))?;
        let client = Client::builder()
            .user_agent("tdw-llm-anthropic/0.1")
            // Bound the client so a stalled endpoint can't hang the eval/agent op
            // forever (IO1). connect_timeout fails fast on an unreachable host;
            // the overall timeout is generous since completions can run long.
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|error| AnthropicHttpError::ClientBuild(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            api_key,
            model_id,
        })
    }

    /// Override the base URL. Useful for tests and for self-hosted
    /// Anthropic-compatible gateways.
    pub fn with_base_url(
        mut self,
        base_url: impl Into<String>,
    ) -> Result<Self, AnthropicHttpError> {
        let raw = base_url.into();
        self.base_url = Url::parse(&raw)
            .map_err(|error| AnthropicHttpError::InvalidBaseUrl(format!("{raw}: {error}")))?;
        Ok(self)
    }

    /// Anthropic model id this client is configured for.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Post the supplied [`ChatRequest`] to `POST /v1/messages` and
    /// translate the response back into a [`ChatResponse`].
    ///
    /// `MessageRole::System` messages become Anthropic's top-level
    /// `system` field (joined with `\n` if multiple). All other roles
    /// (`User`, `Assistant`, `Tool`) go into the `messages` array.
    /// `Tool` is currently mapped to Anthropic's `user` role with a
    /// `[tool] ` prefix; full tool-use support is a follow-up.
    pub async fn complete(&self, request: ChatRequest) -> Result<ChatResponse, AnthropicHttpError> {
        validate_chat_request(&request).map_err(AnthropicHttpError::InvalidRequest)?;
        let body = build_request_body(&self.model_id, &request);
        let response = self
            .client
            .post(messages_url(&self.base_url)?)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AnthropicHttpError::Http { status, body });
        }
        let envelope: MessagesEnvelope = response.json().await?;
        parse_response(&self.model_id, envelope)
    }

    /// Post the supplied [`ChatRequest`] with `stream: true`, call
    /// `on_delta` for each text delta received over SSE, and return
    /// the accumulated final [`ChatResponse`].
    ///
    /// Unknown Anthropic stream event types are ignored as required
    /// by Anthropic's versioning policy; `error` events and malformed
    /// payloads become typed errors.
    pub async fn complete_streaming(
        &self,
        request: ChatRequest,
        mut on_delta: impl FnMut(&str),
    ) -> Result<ChatResponse, AnthropicHttpError> {
        validate_chat_request(&request).map_err(AnthropicHttpError::InvalidRequest)?;
        let mut body = build_request_body(&self.model_id, &request);
        body["stream"] = Value::Bool(true);
        let mut response = self
            .client
            .post(messages_url(&self.base_url)?)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AnthropicHttpError::Http { status, body });
        }

        let mut decoder = SseDecoder::default();
        let mut stream = AnthropicStreamState::new(&self.model_id);
        while let Some(chunk) = response.chunk().await? {
            for event in decoder.push(&chunk)? {
                stream.apply_event(event, &mut on_delta)?;
            }
        }
        for event in decoder.finish()? {
            stream.apply_event(event, &mut on_delta)?;
        }
        stream.finish()
    }
}

fn messages_url(base_url: &Url) -> Result<Url, AnthropicHttpError> {
    base_url
        .join("/v1/messages")
        .map_err(|error| AnthropicHttpError::InvalidBaseUrl(error.to_string()))
}

fn normalize_api_key(api_key: String) -> Result<String, AnthropicHttpError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(AnthropicHttpError::MissingApiKey);
    }
    Ok(api_key.to_string())
}

/// Build the JSON body posted to Anthropic. Extracted into a
/// standalone fn so tests can assert the on-the-wire shape without
/// hitting the network.
pub(crate) fn build_request_body(model_id: &str, request: &ChatRequest) -> Value {
    let mut system_parts = Vec::new();
    let mut messages = Vec::with_capacity(request.messages.len());
    for message in &request.messages {
        match message.role {
            MessageRole::System => {
                system_parts.push(message.content.as_str());
            }
            MessageRole::User => {
                messages.push(json!({
                    "role": "user",
                    "content": message.content,
                }));
            }
            MessageRole::Assistant => {
                messages.push(json!({
                    "role": "assistant",
                    "content": message.content,
                }));
            }
            MessageRole::Tool => {
                // Anthropic's tool-use envelope is richer; for the
                // first slice we fold tool turns into a user message
                // with a marker prefix so the model still sees the
                // content. Full tool-use is a follow-up.
                messages.push(json!({
                    "role": "user",
                    "content": format!("[tool] {}", message.content),
                }));
            }
        }
    }
    let mut body = json!({
        "model": model_id,
        "max_tokens": request.max_output_tokens,
        "messages": messages,
    });
    if !system_parts.is_empty() {
        body["system"] = Value::String(system_parts.join("\n"));
    }
    body
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SseEvent {
    event_type: Option<String>,
    data: String,
}

#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, AnthropicHttpError> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((index, delimiter_len)) = find_sse_boundary(&self.pending) {
            let event_bytes = self.pending[..index].to_vec();
            self.pending.drain(..index + delimiter_len);
            if let Some(event) = parse_sse_event(&event_bytes)? {
                events.push(event);
            }
        }
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<SseEvent>, AnthropicHttpError> {
        if self.pending.iter().all(u8::is_ascii_whitespace) {
            self.pending.clear();
            return Ok(Vec::new());
        }
        let event_bytes = std::mem::take(&mut self.pending);
        Ok(parse_sse_event(&event_bytes)?.into_iter().collect())
    }
}

fn find_sse_boundary(bytes: &[u8]) -> Option<(usize, usize)> {
    for index in 0..bytes.len() {
        if bytes.get(index..index + 4) == Some(b"\r\n\r\n") {
            return Some((index, 4));
        }
        if bytes.get(index..index + 2) == Some(b"\n\n") {
            return Some((index, 2));
        }
    }
    None
}

fn parse_sse_event(bytes: &[u8]) -> Result<Option<SseEvent>, AnthropicHttpError> {
    let raw = std::str::from_utf8(bytes).map_err(|error| {
        AnthropicHttpError::InvalidResponse(format!("sse event was not utf-8: {error}"))
    })?;
    let mut event_type = None;
    let mut data = String::new();
    for line in raw.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event_type = Some(value.to_string()),
            "data" => {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value);
            }
            _ => {}
        }
    }
    if event_type.is_none() && data.is_empty() {
        return Ok(None);
    }
    Ok(Some(SseEvent { event_type, data }))
}

#[derive(Deserialize)]
pub(crate) struct MessagesEnvelope {
    #[serde(default)]
    content: Vec<MessagesBlock>,
    #[serde(default)]
    usage: Option<MessagesUsage>,
}

#[derive(Deserialize)]
pub(crate) struct MessagesBlock {
    #[serde(rename = "type", default)]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct MessagesUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Deserialize)]
pub(crate) struct StreamEnvelope {
    #[serde(rename = "type", default)]
    event_type: String,
    #[serde(default)]
    message: Option<StreamMessage>,
    #[serde(default)]
    delta: Option<StreamDelta>,
    #[serde(default)]
    usage: Option<MessagesUsage>,
    #[serde(default)]
    error: Option<StreamError>,
}

#[derive(Deserialize)]
pub(crate) struct StreamMessage {
    #[serde(default)]
    usage: Option<MessagesUsage>,
}

#[derive(Deserialize)]
pub(crate) struct StreamDelta {
    #[serde(rename = "type", default)]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct StreamError {
    #[serde(rename = "type", default)]
    error_type: String,
    #[serde(default)]
    message: String,
}

struct AnthropicStreamState {
    model_id: String,
    text: String,
    input_tokens: u32,
    output_tokens: u32,
    saw_stop: bool,
}

impl AnthropicStreamState {
    fn new(model_id: &str) -> Self {
        Self {
            model_id: model_id.to_string(),
            text: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            saw_stop: false,
        }
    }

    fn apply_event(
        &mut self,
        event: SseEvent,
        on_delta: &mut impl FnMut(&str),
    ) -> Result<(), AnthropicHttpError> {
        let envelope: StreamEnvelope = serde_json::from_str(&event.data).map_err(|error| {
            AnthropicHttpError::InvalidResponse(format!("anthropic stream json: {error}"))
        })?;
        let event_type = if envelope.event_type.is_empty() {
            event.event_type.as_deref().unwrap_or_default()
        } else {
            envelope.event_type.as_str()
        };
        match event_type {
            "message_start" => {
                if let Some(usage) = envelope.message.and_then(|message| message.usage) {
                    self.input_tokens = usage.input_tokens;
                    self.output_tokens = usage.output_tokens;
                }
            }
            "content_block_delta" => {
                if let Some(delta) = envelope.delta
                    && delta.delta_type == "text_delta"
                {
                    let text = delta.text.unwrap_or_default();
                    if !text.is_empty() {
                        on_delta(&text);
                        self.text.push_str(&text);
                    }
                }
            }
            "message_delta" => {
                if let Some(usage) = envelope.usage {
                    if usage.input_tokens > 0 {
                        self.input_tokens = usage.input_tokens;
                    }
                    self.output_tokens = usage.output_tokens;
                }
            }
            "message_stop" => {
                self.saw_stop = true;
            }
            "error" => {
                let error = envelope.error.ok_or_else(|| {
                    AnthropicHttpError::InvalidResponse(
                        "anthropic stream error event had no error object".to_string(),
                    )
                })?;
                return Err(AnthropicHttpError::InvalidResponse(format!(
                    "anthropic stream error {}: {}",
                    error.error_type, error.message
                )));
            }
            "ping" | "content_block_start" | "content_block_stop" => {}
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<ChatResponse, AnthropicHttpError> {
        let content = self.text.trim().to_string();
        if content.is_empty() {
            return Err(AnthropicHttpError::InvalidResponse(
                "anthropic stream had no text deltas".to_string(),
            ));
        }
        if !self.saw_stop {
            return Err(AnthropicHttpError::InvalidResponse(
                "anthropic stream ended before message_stop".to_string(),
            ));
        }
        Ok(ChatResponse {
            model_id: self.model_id,
            message: ChatMessage {
                role: MessageRole::Assistant,
                content,
            },
            usage: Usage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
            },
        })
    }
}

/// Parse the Anthropic Messages response envelope into a workspace
/// [`ChatResponse`]. Extracted so cassette tests can exercise the
/// decoder without touching the network.
pub(crate) fn parse_response(
    model_id: &str,
    envelope: MessagesEnvelope,
) -> Result<ChatResponse, AnthropicHttpError> {
    let text = envelope
        .content
        .into_iter()
        .filter_map(|block| {
            if block.block_type == "text" {
                block.text
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("");
    if text.is_empty() {
        return Err(AnthropicHttpError::InvalidResponse(
            "anthropic response had no text content blocks".to_string(),
        ));
    }
    let usage = envelope.usage.unwrap_or(MessagesUsage {
        input_tokens: 0,
        output_tokens: 0,
    });
    Ok(ChatResponse {
        model_id: model_id.to_string(),
        message: ChatMessage {
            role: MessageRole::Assistant,
            content: text,
        },
        usage: Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_constructor_rejects_empty_key_and_debug_redacts_key() {
        assert!(matches!(
            AnthropicHttpClient::new("", "claude-haiku-4-5-20251001"),
            Err(AnthropicHttpError::MissingApiKey)
        ));
        let client = AnthropicHttpClient::new("sk-ant-test", "claude-haiku-4-5-20251001")
            .unwrap_or_else(|error| panic!("client should build: {error}"));
        let debug = format!("{client:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("sk-ant-test"));
    }

    #[test]
    fn system_message_becomes_top_level_system_field_and_user_goes_in_messages() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: "be terse".to_string(),
                },
                ChatMessage {
                    role: MessageRole::User,
                    content: "hello".to_string(),
                },
            ],
            max_output_tokens: 32,
        };
        let body = build_request_body("claude-haiku-4-5-20251001", &request);
        assert_eq!(body["model"], "claude-haiku-4-5-20251001");
        assert_eq!(body["max_tokens"], 32);
        assert_eq!(body["system"], "be terse");
        let messages = body["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hello");
    }

    #[test]
    fn multiple_system_messages_join_with_newline() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: "policy a".to_string(),
                },
                ChatMessage {
                    role: MessageRole::System,
                    content: "policy b".to_string(),
                },
                ChatMessage {
                    role: MessageRole::User,
                    content: "go".to_string(),
                },
            ],
            max_output_tokens: 8,
        };
        let body = build_request_body("claude-haiku-4-5-20251001", &request);
        assert_eq!(body["system"], "policy a\npolicy b");
    }

    #[test]
    fn tool_role_is_folded_into_user_message_with_marker() {
        let request = ChatRequest {
            messages: vec![
                ChatMessage {
                    role: MessageRole::Tool,
                    content: "result=42".to_string(),
                },
                ChatMessage {
                    role: MessageRole::User,
                    content: "summarise".to_string(),
                },
            ],
            max_output_tokens: 8,
        };
        let body = build_request_body("claude-haiku-4-5-20251001", &request);
        let messages = body["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "[tool] result=42");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "summarise");
    }

    #[test]
    fn cassette_replay_decodes_messages_response() {
        let raw = r#"{
            "id": "msg_test",
            "type": "message",
            "role": "assistant",
            "model": "claude-haiku-4-5-20251001",
            "content": [
                { "type": "text", "text": "Hello there." }
            ],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 17, "output_tokens": 5 }
        }"#;
        let envelope: MessagesEnvelope = serde_json::from_str(raw).expect("envelope must parse");
        let response =
            parse_response("claude-haiku-4-5-20251001", envelope).expect("envelope must decode");
        assert_eq!(response.model_id, "claude-haiku-4-5-20251001");
        assert_eq!(response.message.role, MessageRole::Assistant);
        assert_eq!(response.message.content, "Hello there.");
        assert_eq!(response.usage.input_tokens, 17);
        assert_eq!(response.usage.output_tokens, 5);
    }

    #[test]
    fn cassette_replay_joins_multiple_text_blocks() {
        let raw = r#"{
            "content": [
                { "type": "text", "text": "First. " },
                { "type": "tool_use", "id": "abc" },
                { "type": "text", "text": "Second." }
            ],
            "usage": { "input_tokens": 3, "output_tokens": 4 }
        }"#;
        let envelope: MessagesEnvelope = serde_json::from_str(raw).expect("envelope must parse");
        let response =
            parse_response("claude-haiku-4-5-20251001", envelope).expect("envelope must decode");
        assert_eq!(response.message.content, "First. Second.");
    }

    #[test]
    fn cassette_replay_errors_when_no_text_content() {
        let raw = r#"{
            "content": [ { "type": "tool_use", "id": "abc" } ],
            "usage": { "input_tokens": 1, "output_tokens": 1 }
        }"#;
        let envelope: MessagesEnvelope = serde_json::from_str(raw).expect("envelope must parse");
        let err = parse_response("claude-haiku-4-5-20251001", envelope)
            .expect_err("empty text must fail");
        assert!(matches!(err, AnthropicHttpError::InvalidResponse(_)));
    }

    #[test]
    fn streaming_request_body_sets_stream_true() {
        let request = ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "hello".to_string(),
            }],
            max_output_tokens: 32,
        };
        let mut body = build_request_body("claude-haiku-4-5-20251001", &request);
        body["stream"] = Value::Bool(true);
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[test]
    fn sse_decoder_handles_split_chunks_and_crlf_boundaries() {
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(b"event: content_block_delta\r\n")
                .expect("partial event should parse")
                .is_empty()
        );
        let events = decoder
            .push(
                br#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"Hel"}}"#,
            )
            .expect("partial event should parse");
        assert!(events.is_empty());
        let events = decoder.push(b"\r\n\r\n").expect("event should parse");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type.as_deref(), Some("content_block_delta"));
        assert!(events[0].data.contains("\"Hel\""));
    }

    #[test]
    fn cassette_replay_decodes_anthropic_stream() {
        let raw = [
            br#"event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":7,"output_tokens":1}}}

"#
            .as_slice(),
            br#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}

event: message_stop
data: {"type":"message_stop"}

"#
            .as_slice(),
        ];
        let mut decoder = SseDecoder::default();
        let mut stream = AnthropicStreamState::new("claude-haiku-4-5-20251001");
        let mut deltas = Vec::new();
        for chunk in raw {
            for event in decoder.push(chunk).expect("chunk should parse") {
                stream
                    .apply_event(event, &mut |delta| deltas.push(delta.to_string()))
                    .expect("stream event should apply");
            }
        }
        for event in decoder.finish().expect("tail should parse") {
            stream
                .apply_event(event, &mut |delta| deltas.push(delta.to_string()))
                .expect("stream event should apply");
        }
        let response = stream.finish().expect("stream should finish");
        assert_eq!(deltas, vec!["Hel", "lo"]);
        assert_eq!(response.message.content, "Hello");
        assert_eq!(response.usage.input_tokens, 7);
        assert_eq!(response.usage.output_tokens, 3);
    }

    #[test]
    fn cassette_replay_errors_on_stream_error_event() {
        let event = SseEvent {
            event_type: Some("error".to_string()),
            data: r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
                .to_string(),
        };
        let mut stream = AnthropicStreamState::new("claude-haiku-4-5-20251001");
        assert!(stream.apply_event(event, &mut |_| {}).is_err());
    }

    #[test]
    fn http_error_classification_covers_transient_permanent_and_overflow() {
        let transient = AnthropicHttpError::Http {
            status: reqwest::StatusCode::TOO_MANY_REQUESTS,
            body: String::new(),
        };
        assert_eq!(transient.retryability(), Retryability::Transient);

        let permanent = AnthropicHttpError::Http {
            status: reqwest::StatusCode::BAD_REQUEST,
            body: "invalid_request_error".to_string(),
        };
        assert_eq!(permanent.retryability(), Retryability::Permanent);

        let overflow = AnthropicHttpError::Http {
            status: reqwest::StatusCode::BAD_REQUEST,
            body: r#"{"error":{"message":"prompt is too long: context length exceeds tokens"}}"#
                .to_string(),
        };
        assert_eq!(overflow.retryability(), Retryability::ContextOverflow);

        let local = AnthropicHttpError::MissingApiKey;
        assert_eq!(local.retryability(), Retryability::Permanent);
    }
}
