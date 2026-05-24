//! Real Anthropic Messages API client for G012.
//!
//! Gated by the `http` feature. Posts to `POST /v1/messages` against
//! Anthropic's public API (or any compatible base URL via
//! [`AnthropicHttpClient::with_base_url`]). The existing sync stub
//! [`crate::AnthropicMessagesModel`] is preserved as-is for offline
//! tests; this client is **async-native** because the workspace's
//! `LanguageModel` trait is synchronous (`fn complete(&self, …)`)
//! and bridging async-over-sync is a separate concern.
//!
//! Streaming (`POST /v1/messages` with `stream: true` SSE) is a
//! follow-up slice; this PR ships the batch endpoint only.

use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tdw_llm::{
    ChatMessage, ChatRequest, ChatResponse, MessageRole, Usage, validate_chat_request,
    validate_model_id,
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

impl AnthropicHttpClient {
    /// Construct a client for a specific Anthropic model id (e.g.
    /// `"claude-haiku-4-5-20251001"`). Defaults to the public API
    /// base URL.
    pub fn new(
        api_key: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Result<Self, AnthropicHttpError> {
        let model_id = model_id.into();
        validate_model_id(&model_id)
            .map_err(|_| AnthropicHttpError::InvalidModelId(model_id.clone()))?;
        let base_url = Url::parse(DEFAULT_BASE_URL)
            .map_err(|error| AnthropicHttpError::InvalidBaseUrl(error.to_string()))?;
        let client = Client::builder()
            .user_agent("tdw-llm-anthropic/0.1")
            .build()
            .map_err(|error| AnthropicHttpError::ClientBuild(error.to_string()))?;
        Ok(Self {
            client,
            base_url,
            api_key: api_key.into(),
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
        let url = self
            .base_url
            .join("/v1/messages")
            .map_err(|error| AnthropicHttpError::InvalidBaseUrl(error.to_string()))?;
        let response = self
            .client
            .post(url)
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
}
