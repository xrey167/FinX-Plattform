//! Real OpenAI-compatible Chat Completions client for G012.
//!
//! Gated by the `http` feature. Posts to
//! `POST /v1/chat/completions` against OpenAI or any compatible
//! gateway via [`OpenAiCompatibleHttpClient::with_base_url`]. The
//! existing sync stub [`crate::OpenAiCompatibleModel`] remains the
//! offline default. This client is async-native because
//! `tdw_llm::LanguageModel` is synchronous; bridging async-over-sync
//! is a separate runtime concern.

use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_llm::{
    ChatMessage, ChatRequest, ChatResponse, ClassifyError, MessageRole, Retryability, Usage,
    classify_http_status, validate_base_url, validate_chat_request, validate_model_id,
};
use thiserror::Error;

const DEFAULT_BASE_URL: &str = "https://api.openai.com";

/// Production OpenAI-compatible Chat Completions client.
#[derive(Clone)]
pub struct OpenAiCompatibleHttpClient {
    client: Client,
    base_url: Url,
    api_key: Option<String>,
    model_id: String,
}

impl std::fmt::Debug for OpenAiCompatibleHttpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiCompatibleHttpClient")
            .field("base_url", &self.base_url.as_str())
            .field("model_id", &self.model_id)
            .field("api_key", &self.api_key.as_ref().map(|_| "REDACTED"))
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum OpenAiCompatibleHttpError {
    #[error("openai-compatible api key must not be empty; use without_api_key for local gateways")]
    MissingApiKey,
    #[error("invalid model id: {0}")]
    InvalidModelId(String),
    #[error("invalid base url: {0}")]
    InvalidBaseUrl(String),
    #[error("chat request invalid: {0}")]
    InvalidRequest(tdw_llm::LlmError),
    #[error("openai-compatible http {status}: {body}")]
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

impl ClassifyError for OpenAiCompatibleHttpError {
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

impl OpenAiCompatibleHttpClient {
    /// Construct a client for an authenticated OpenAI-compatible
    /// endpoint. Defaults to `https://api.openai.com` and appends the
    /// `/v1/chat/completions` path when completing a request.
    pub fn new(
        api_key: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, OpenAiCompatibleHttpError> {
        let api_key = normalize_api_key(api_key.into())?;
        Self::build(Some(api_key), model_id.into())
    }

    /// Construct a client for a local or private compatible gateway
    /// that intentionally does not require bearer authentication.
    pub fn without_api_key(model_id: impl Into<String>) -> Result<Self, OpenAiCompatibleHttpError> {
        Self::build(None, model_id.into())
    }

    fn build(api_key: Option<String>, model_id: String) -> Result<Self, OpenAiCompatibleHttpError> {
        validate_model_id(&model_id)
            .map_err(|_| OpenAiCompatibleHttpError::InvalidModelId(model_id.clone()))?;
        let base_url = parse_base_url(DEFAULT_BASE_URL)?;
        let client = Client::builder()
            .user_agent("tdw-llm-openai-compat/0.1")
            // Bound the client so a stalled endpoint can't hang the eval/agent op
            // forever (IO1). connect_timeout fails fast on an unreachable host;
            // the overall timeout is generous since completions can run long.
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|error| OpenAiCompatibleHttpError::ClientBuild(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            api_key,
            model_id,
        })
    }

    /// Override the base URL. Supplying either a host root (for
    /// example `http://localhost:11434`) or an existing `/v1` base is
    /// accepted; the client normalizes both to
    /// `/v1/chat/completions`.
    pub fn with_base_url(
        mut self,
        base_url: impl Into<String>,
    ) -> Result<Self, OpenAiCompatibleHttpError> {
        self.base_url = parse_base_url(&base_url.into())?;
        Ok(self)
    }

    /// OpenAI-compatible model id this client is configured for.
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    /// Post the supplied [`ChatRequest`] to
    /// `POST /v1/chat/completions` and translate the first assistant
    /// choice into a [`ChatResponse`].
    ///
    /// `MessageRole::Tool` is currently mapped to a `user` message
    /// with a `[tool] ` prefix. Full Chat Completions tool-call
    /// envelopes are a follow-up.
    pub async fn complete(
        &self,
        request: ChatRequest,
    ) -> Result<ChatResponse, OpenAiCompatibleHttpError> {
        validate_chat_request(&request).map_err(OpenAiCompatibleHttpError::InvalidRequest)?;
        let body = build_request_body(&self.model_id, &request);
        let mut builder = self
            .client
            .post(chat_completions_url(&self.base_url)?)
            .header("content-type", "application/json")
            .json(&body);
        if let Some(api_key) = self.api_key.as_deref() {
            builder = builder.bearer_auth(api_key);
        }
        let response = builder.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OpenAiCompatibleHttpError::Http { status, body });
        }
        let envelope: ChatCompletionEnvelope = response.json().await?;
        parse_response(&self.model_id, envelope)
    }

    /// Post the supplied [`ChatRequest`] with `stream: true`, call
    /// `on_delta` for each `choices[].delta.content` chunk received
    /// over SSE, and return the accumulated final [`ChatResponse`].
    ///
    /// Usage is populated when the upstream gateway emits a streaming
    /// usage chunk. Gateways that omit usage still return the text
    /// stream with zero usage counts.
    pub async fn complete_streaming(
        &self,
        request: ChatRequest,
        mut on_delta: impl FnMut(&str),
    ) -> Result<ChatResponse, OpenAiCompatibleHttpError> {
        validate_chat_request(&request).map_err(OpenAiCompatibleHttpError::InvalidRequest)?;
        let mut body = build_request_body(&self.model_id, &request);
        body["stream"] = Value::Bool(true);
        body["stream_options"] = json!({ "include_usage": true });
        let mut builder = self
            .client
            .post(chat_completions_url(&self.base_url)?)
            .header("content-type", "application/json")
            .json(&body);
        if let Some(api_key) = self.api_key.as_deref() {
            builder = builder.bearer_auth(api_key);
        }
        let mut response = builder.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OpenAiCompatibleHttpError::Http { status, body });
        }

        let mut decoder = SseDecoder::default();
        let mut stream = OpenAiStreamState::new(&self.model_id);
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

fn normalize_api_key(api_key: String) -> Result<String, OpenAiCompatibleHttpError> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err(OpenAiCompatibleHttpError::MissingApiKey);
    }
    Ok(api_key.to_string())
}

fn parse_base_url(base_url: &str) -> Result<Url, OpenAiCompatibleHttpError> {
    validate_base_url(base_url)
        .map_err(|error| OpenAiCompatibleHttpError::InvalidBaseUrl(error.to_string()))?;
    Url::parse(base_url)
        .map_err(|error| OpenAiCompatibleHttpError::InvalidBaseUrl(error.to_string()))
}

fn chat_completions_url(base_url: &Url) -> Result<Url, OpenAiCompatibleHttpError> {
    let mut url = base_url.clone();
    let path = url.path().trim_end_matches('/');
    let path = if path.ends_with("/v1/chat/completions") {
        path.to_string()
    } else if path.ends_with("/v1") {
        format!("{path}/chat/completions")
    } else if path.is_empty() {
        "/v1/chat/completions".to_string()
    } else {
        format!("{path}/v1/chat/completions")
    };
    url.set_path(&path);
    url.set_query(None);
    Ok(url)
}

/// Build the JSON body posted to OpenAI-compatible Chat Completions.
/// Extracted so tests can assert the wire shape without touching the
/// network.
pub(crate) fn build_request_body(model_id: &str, request: &ChatRequest) -> Value {
    let messages = request
        .messages
        .iter()
        .map(|message| match message.role {
            MessageRole::System => json!({
                "role": "system",
                "content": message.content,
            }),
            MessageRole::User => json!({
                "role": "user",
                "content": message.content,
            }),
            MessageRole::Assistant => json!({
                "role": "assistant",
                "content": message.content,
            }),
            MessageRole::Tool => json!({
                "role": "user",
                "content": format!("[tool] {}", message.content),
            }),
        })
        .collect::<Vec<_>>();
    json!({
        "model": model_id,
        "max_tokens": request.max_output_tokens,
        "messages": messages,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SseEvent {
    data: String,
}

#[derive(Default)]
struct SseDecoder {
    pending: Vec<u8>,
}

impl SseDecoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, OpenAiCompatibleHttpError> {
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

    fn finish(&mut self) -> Result<Vec<SseEvent>, OpenAiCompatibleHttpError> {
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

fn parse_sse_event(bytes: &[u8]) -> Result<Option<SseEvent>, OpenAiCompatibleHttpError> {
    let raw = std::str::from_utf8(bytes).map_err(|error| {
        OpenAiCompatibleHttpError::InvalidResponse(format!("sse event was not utf-8: {error}"))
    })?;
    let mut data = String::new();
    for line in raw.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        if field != "data" {
            continue;
        }
        let value = value.strip_prefix(' ').unwrap_or(value);
        if !data.is_empty() {
            data.push('\n');
        }
        data.push_str(value);
    }
    if data.is_empty() {
        return Ok(None);
    }
    Ok(Some(SseEvent { data }))
}

#[derive(Deserialize)]
pub(crate) struct ChatCompletionEnvelope {
    #[serde(default)]
    choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    usage: Option<ChatCompletionUsage>,
}

#[derive(Deserialize)]
pub(crate) struct ChatCompletionChoice {
    #[serde(default)]
    message: Option<ChatCompletionMessage>,
}

#[derive(Deserialize)]
pub(crate) struct ChatCompletionMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ChatCompletionUsage {
    #[serde(default, alias = "input_tokens")]
    prompt_tokens: u32,
    #[serde(default, alias = "output_tokens")]
    completion_tokens: u32,
}

#[derive(Deserialize)]
pub(crate) struct StreamCompletionEnvelope {
    #[serde(default)]
    choices: Vec<StreamCompletionChoice>,
    #[serde(default)]
    usage: Option<ChatCompletionUsage>,
    #[serde(default)]
    error: Option<StreamError>,
}

#[derive(Deserialize)]
pub(crate) struct StreamCompletionChoice {
    #[serde(default)]
    delta: StreamCompletionDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
pub(crate) struct StreamCompletionDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct StreamError {
    #[serde(rename = "type", default)]
    error_type: String,
    #[serde(default)]
    message: String,
}

struct OpenAiStreamState {
    model_id: String,
    text: String,
    input_tokens: u32,
    output_tokens: u32,
    saw_done: bool,
    saw_finish_reason: bool,
}

impl OpenAiStreamState {
    fn new(model_id: &str) -> Self {
        Self {
            model_id: model_id.to_string(),
            text: String::new(),
            input_tokens: 0,
            output_tokens: 0,
            saw_done: false,
            saw_finish_reason: false,
        }
    }

    fn apply_event(
        &mut self,
        event: SseEvent,
        on_delta: &mut impl FnMut(&str),
    ) -> Result<(), OpenAiCompatibleHttpError> {
        if event.data.trim() == "[DONE]" {
            self.saw_done = true;
            return Ok(());
        }
        let envelope: StreamCompletionEnvelope =
            serde_json::from_str(&event.data).map_err(|error| {
                OpenAiCompatibleHttpError::InvalidResponse(format!(
                    "openai-compatible stream json: {error}"
                ))
            })?;
        if let Some(error) = envelope.error {
            return Err(OpenAiCompatibleHttpError::InvalidResponse(format!(
                "openai-compatible stream error {}: {}",
                error.error_type, error.message
            )));
        }
        if let Some(usage) = envelope.usage {
            self.input_tokens = usage.prompt_tokens;
            self.output_tokens = usage.completion_tokens;
        }
        for choice in envelope.choices {
            if choice.finish_reason.is_some() {
                self.saw_finish_reason = true;
            }
            if let Some(content) = choice.delta.content
                && !content.is_empty()
            {
                on_delta(&content);
                self.text.push_str(&content);
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<ChatResponse, OpenAiCompatibleHttpError> {
        let content = self.text.trim().to_string();
        if content.is_empty() {
            return Err(OpenAiCompatibleHttpError::InvalidResponse(
                "openai-compatible stream had no text deltas".to_string(),
            ));
        }
        if !self.saw_done && !self.saw_finish_reason {
            return Err(OpenAiCompatibleHttpError::InvalidResponse(
                "openai-compatible stream ended before [DONE] or finish_reason".to_string(),
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

/// Parse the Chat Completions response envelope into a workspace
/// [`ChatResponse`]. Extracted so cassette tests can exercise the
/// decoder without touching the network.
pub(crate) fn parse_response(
    model_id: &str,
    envelope: ChatCompletionEnvelope,
) -> Result<ChatResponse, OpenAiCompatibleHttpError> {
    let content = envelope
        .choices
        .into_iter()
        .find_map(|choice| choice.message.and_then(|message| message.content))
        .map(|content| content.trim().to_string())
        .filter(|content| !content.is_empty())
        .ok_or_else(|| {
            OpenAiCompatibleHttpError::InvalidResponse(
                "openai-compatible response had no assistant text content".to_string(),
            )
        })?;
    let usage = envelope.usage.unwrap_or(ChatCompletionUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
    });
    Ok(ChatResponse {
        model_id: model_id.to_string(),
        message: ChatMessage {
            role: MessageRole::Assistant,
            content,
        },
        usage: Usage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_constructor_rejects_empty_key_and_debug_redacts_key() {
        assert!(matches!(
            OpenAiCompatibleHttpClient::new("", "gpt-4o-mini"),
            Err(OpenAiCompatibleHttpError::MissingApiKey)
        ));
        let client = OpenAiCompatibleHttpClient::new("sk-test", "gpt-4o-mini")
            .unwrap_or_else(|error| panic!("client should build: {error}"));
        let debug = format!("{client:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("sk-test"));
    }

    #[test]
    fn endpoint_url_normalizes_host_root_and_existing_v1_base() {
        let host_root = OpenAiCompatibleHttpClient::without_api_key("llama3")
            .unwrap_or_else(|error| panic!("client should build: {error}"))
            .with_base_url("http://localhost:11434")
            .unwrap_or_else(|error| panic!("base url should parse: {error}"));
        let existing_v1 = OpenAiCompatibleHttpClient::without_api_key("llama3")
            .unwrap_or_else(|error| panic!("client should build: {error}"))
            .with_base_url("http://localhost:11434/v1")
            .unwrap_or_else(|error| panic!("base url should parse: {error}"));

        assert_eq!(
            chat_completions_url(&host_root.base_url)
                .unwrap_or_else(|error| panic!("url should build: {error}"))
                .as_str(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url(&existing_v1.base_url)
                .unwrap_or_else(|error| panic!("url should build: {error}"))
                .as_str(),
            "http://localhost:11434/v1/chat/completions"
        );
        assert!(
            OpenAiCompatibleHttpClient::without_api_key("llama3")
                .unwrap_or_else(|error| panic!("client should build: {error}"))
                .with_base_url("localhost:11434")
                .is_err()
        );
    }

    #[test]
    fn request_body_maps_roles_and_max_tokens() {
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
                ChatMessage {
                    role: MessageRole::Assistant,
                    content: "hi".to_string(),
                },
                ChatMessage {
                    role: MessageRole::Tool,
                    content: "result=42".to_string(),
                },
            ],
            max_output_tokens: 32,
        };
        let body = build_request_body("gpt-4o-mini", &request);
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["max_tokens"], 32);
        let Some(messages) = body["messages"].as_array() else {
            panic!("messages must be an array");
        };
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[3]["role"], "user");
        assert_eq!(messages[3]["content"], "[tool] result=42");
    }

    #[test]
    fn cassette_replay_decodes_chat_completions_response() {
        let raw = r#"{
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [
                {
                    "index": 0,
                    "message": { "role": "assistant", "content": "Hello there." },
                    "finish_reason": "stop"
                }
            ],
            "usage": { "prompt_tokens": 17, "completion_tokens": 5, "total_tokens": 22 }
        }"#;
        let envelope: ChatCompletionEnvelope =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("envelope parses: {error}"));
        let response =
            parse_response("gpt-4o-mini", envelope).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(response.model_id, "gpt-4o-mini");
        assert_eq!(response.message.role, MessageRole::Assistant);
        assert_eq!(response.message.content, "Hello there.");
        assert_eq!(response.usage.input_tokens, 17);
        assert_eq!(response.usage.output_tokens, 5);
    }

    #[test]
    fn cassette_replay_supports_compat_usage_aliases() {
        let raw = r#"{
            "choices": [
                { "message": { "content": "Alias usage." } }
            ],
            "usage": { "input_tokens": 3, "output_tokens": 4 }
        }"#;
        let envelope: ChatCompletionEnvelope =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("envelope parses: {error}"));
        let response =
            parse_response("compat-model", envelope).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(response.usage.input_tokens, 3);
        assert_eq!(response.usage.output_tokens, 4);
    }

    #[test]
    fn cassette_replay_errors_when_no_assistant_text_content() {
        let raw = r#"{
            "choices": [
                { "message": { "content": null } }
            ],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1 }
        }"#;
        let envelope: ChatCompletionEnvelope =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("envelope parses: {error}"));
        let err = parse_response("gpt-4o-mini", envelope).err();
        assert!(matches!(
            err,
            Some(OpenAiCompatibleHttpError::InvalidResponse(_))
        ));
    }

    #[test]
    fn streaming_request_body_sets_stream_true_and_usage_options() {
        let request = ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "hello".to_string(),
            }],
            max_output_tokens: 32,
        };
        let mut body = build_request_body("gpt-4o-mini", &request);
        body["stream"] = Value::Bool(true);
        body["stream_options"] = json!({ "include_usage": true });
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[test]
    fn sse_decoder_handles_split_chunks_and_data_only_events() {
        let mut decoder = SseDecoder::default();
        assert!(
            decoder
                .push(b"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]")
                .expect("partial event should parse")
                .is_empty()
        );
        let events = decoder.push(b"}\n\n").expect("event should parse");
        assert_eq!(events.len(), 1);
        assert!(events[0].data.contains("\"Hel\""));
        let done = decoder
            .push(b"data: [DONE]\r\n\r\n")
            .expect("done should parse");
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].data, "[DONE]");
    }

    #[test]
    fn cassette_replay_decodes_openai_stream() {
        let raw = br#"data: {"choices":[{"delta":{"role":"assistant","content":""}}]}

data: {"choices":[{"delta":{"content":"Hel"}}]}

data: {"choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}]}

data: {"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}

data: [DONE]

"#;
        let mut decoder = SseDecoder::default();
        let mut stream = OpenAiStreamState::new("gpt-4o-mini");
        let mut deltas = Vec::new();
        for event in decoder.push(raw).expect("stream should parse") {
            stream
                .apply_event(event, &mut |delta| deltas.push(delta.to_string()))
                .expect("stream event should apply");
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
            data: r#"{"error":{"type":"invalid_request_error","message":"bad stream"}}"#
                .to_string(),
        };
        let mut stream = OpenAiStreamState::new("gpt-4o-mini");
        assert!(stream.apply_event(event, &mut |_| {}).is_err());
    }

    #[test]
    fn http_error_classification_covers_transient_permanent_and_overflow() {
        let transient = OpenAiCompatibleHttpError::Http {
            status: reqwest::StatusCode::SERVICE_UNAVAILABLE,
            body: String::new(),
        };
        assert_eq!(transient.retryability(), Retryability::Transient);

        let permanent = OpenAiCompatibleHttpError::Http {
            status: reqwest::StatusCode::UNAUTHORIZED,
            body: "invalid api key".to_string(),
        };
        assert_eq!(permanent.retryability(), Retryability::Permanent);

        let overflow = OpenAiCompatibleHttpError::Http {
            status: reqwest::StatusCode::BAD_REQUEST,
            body: r#"{"error":{"message":"maximum context length is 8192 tokens"}}"#.to_string(),
        };
        assert_eq!(overflow.retryability(), Retryability::ContextOverflow);

        let local = OpenAiCompatibleHttpError::MissingApiKey;
        assert_eq!(local.retryability(), Retryability::Permanent);
    }
}
