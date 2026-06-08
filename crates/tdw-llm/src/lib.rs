#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_config::TdwConfig;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, LlmError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LlmError {
    #[error("chat request has no messages")]
    EmptyMessages,
    #[error("chat message content is empty")]
    EmptyMessageContent,
    #[error("chat request max_output_tokens must be greater than zero")]
    EmptyMaxOutputTokens,
    #[error("model id must not be empty")]
    EmptyModelId,
    #[error("model id contains unsupported control characters")]
    InvalidModelId,
    #[error("base url must start with http:// or https://")]
    InvalidBaseUrl,
    #[error("base url contains whitespace or control characters")]
    UnsafeBaseUrl,
    #[error("unsupported provider: {0}")]
    UnsupportedProvider(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub max_output_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatResponse {
    pub model_id: String,
    pub message: ChatMessage,
    pub usage: Usage,
}

pub trait LanguageModel: Send + Sync {
    fn model_id(&self) -> &str;
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn complete(&self, request: ChatRequest) -> Result<ChatResponse>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
}

impl ModelSelection {
    #[must_use]
    pub fn from_config(config: &TdwConfig) -> Self {
        Self {
            provider: config.model.provider.clone(),
            model: config.model.model.clone(),
            base_url: config.model.base_url.clone(),
        }
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn last_user_message(request: &ChatRequest) -> Result<&str> {
    validate_chat_request(request)?;
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .map(|message| message.content.as_str())
        .ok_or(LlmError::EmptyMessages)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_chat_request(request: &ChatRequest) -> Result<()> {
    if request.messages.is_empty() {
        return Err(LlmError::EmptyMessages);
    }
    if request.max_output_tokens == 0 {
        return Err(LlmError::EmptyMaxOutputTokens);
    }
    if request
        .messages
        .iter()
        .any(|message| message.content.trim().is_empty())
    {
        return Err(LlmError::EmptyMessageContent);
    }
    Ok(())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_model_id(model_id: &str) -> Result<()> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err(LlmError::EmptyModelId);
    }
    if model_id.chars().any(char::is_control) {
        return Err(LlmError::InvalidModelId);
    }
    Ok(())
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_base_url(base_url: &str) -> Result<()> {
    if base_url
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(LlmError::UnsafeBaseUrl);
    }
    if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
        return Err(LlmError::InvalidBaseUrl);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoModel;

    impl LanguageModel for EchoModel {
        fn model_id(&self) -> &'static str {
            "echo"
        }

        fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
            let content = last_user_message(&request)?.to_string();
            Ok(ChatResponse {
                model_id: self.model_id().to_string(),
                message: ChatMessage {
                    role: MessageRole::Assistant,
                    content,
                },
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }
    }

    #[test]
    fn model_trait_completes_chat_request() {
        let model = EchoModel;
        let response = model
            .complete(ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: "hello".to_string(),
                }],
                max_output_tokens: 32,
            })
            .unwrap_or_else(|error| panic!("model completes: {error}"));

        assert_eq!(response.model_id, "echo");
        assert_eq!(response.message.content, "hello");
        assert_eq!(
            last_user_message(&ChatRequest {
                messages: vec![
                    ChatMessage {
                        role: MessageRole::System,
                        content: "policy".to_string(),
                    },
                    ChatMessage {
                        role: MessageRole::User,
                        content: "latest".to_string(),
                    },
                ],
                max_output_tokens: 8,
            }),
            Ok("latest")
        );
        assert!(
            model
                .complete(ChatRequest {
                    messages: vec![ChatMessage {
                        role: MessageRole::User,
                        content: "hello".to_string(),
                    }],
                    max_output_tokens: 0,
                })
                .is_err()
        );
        assert!(validate_model_id("bad\nmodel").is_err());
        assert!(validate_base_url("https://api.example.com/v1").is_ok());
        assert!(validate_base_url("https://api.example.com/v1 token").is_err());
    }

    #[test]
    fn validation_reports_each_remaining_error_variant() {
        assert_eq!(
            validate_chat_request(&ChatRequest {
                messages: vec![],
                max_output_tokens: 8,
            }),
            Err(LlmError::EmptyMessages)
        );
        assert_eq!(
            validate_chat_request(&ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: "  ".to_string(),
                }],
                max_output_tokens: 8,
            }),
            Err(LlmError::EmptyMessageContent)
        );

        assert_eq!(validate_model_id("   "), Err(LlmError::EmptyModelId));

        // A non-http(s) scheme without whitespace is InvalidBaseUrl, distinct from
        // the whitespace/control UnsafeBaseUrl path.
        assert_eq!(
            validate_base_url("ftp://api.example.com"),
            Err(LlmError::InvalidBaseUrl)
        );

        // No user turn present -> EmptyMessages from last_user_message.
        assert_eq!(
            last_user_message(&ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::System,
                    content: "policy".to_string(),
                }],
                max_output_tokens: 8,
            }),
            Err(LlmError::EmptyMessages)
        );
    }
}
