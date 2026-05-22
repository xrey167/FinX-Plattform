#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_config::TdwConfig;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, LlmError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LlmError {
    #[error("chat request has no messages")]
    EmptyMessages,
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
    fn complete(&self, request: ChatRequest) -> Result<ChatResponse>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
    pub base_url: Option<String>,
}

impl ModelSelection {
    pub fn from_config(config: &TdwConfig) -> Self {
        Self {
            provider: config.model.provider.clone(),
            model: config.model.model.clone(),
            base_url: config.model.base_url.clone(),
        }
    }
}

pub fn last_user_message(request: &ChatRequest) -> Result<&str> {
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .map(|message| message.content.as_str())
        .ok_or(LlmError::EmptyMessages)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoModel;

    impl LanguageModel for EchoModel {
        fn model_id(&self) -> &str {
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
    }
}
