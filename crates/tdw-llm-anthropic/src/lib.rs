#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_client;

#[cfg(feature = "http")]
pub use http_client::{AnthropicHttpClient, AnthropicHttpError};

use tdw_llm::{
    ChatMessage, ChatRequest, ChatResponse, LanguageModel, MessageRole, Result, Usage,
    last_user_message, validate_model_id,
};

#[derive(Clone, Debug)]
pub struct AnthropicMessagesModel {
    model_id: String,
}

impl AnthropicMessagesModel {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(model_id: impl Into<String>) -> Result<Self> {
        let model_id = model_id.into();
        validate_model_id(&model_id)?;
        Ok(Self { model_id })
    }
}

impl LanguageModel for AnthropicMessagesModel {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        let prompt = last_user_message(&request)?;
        Ok(ChatResponse {
            model_id: self.model_id.clone(),
            message: ChatMessage {
                role: MessageRole::Assistant,
                content: format!("anthropic:{}:{prompt}", self.model_id),
            },
            usage: Usage {
                input_tokens: u32::try_from(request.messages.len()).unwrap_or(u32::MAX),
                output_tokens: u32::try_from(prompt.split_whitespace().count()).unwrap_or(u32::MAX),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_adapter_implements_language_model_contract() {
        let model = AnthropicMessagesModel::new("claude-test")
            .unwrap_or_else(|error| panic!("model id should be valid: {error}"));
        let response = model
            .complete(ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: "summarize AAPL".to_string(),
                }],
                max_output_tokens: 128,
            })
            .unwrap_or_else(|error| panic!("model completes: {error}"));

        assert_eq!(response.model_id, "claude-test");
        assert!(response.message.content.contains("summarize AAPL"));
        assert!(AnthropicMessagesModel::new("").is_err());
        assert!(AnthropicMessagesModel::new("claude\nbad").is_err());
        assert!(
            model
                .complete(ChatRequest {
                    messages: vec![ChatMessage {
                        role: MessageRole::User,
                        content: "   ".to_string(),
                    }],
                    max_output_tokens: 128,
                })
                .is_err()
        );
    }
}
