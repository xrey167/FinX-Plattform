#![forbid(unsafe_code)]

use tdw_llm::{
    ChatMessage, ChatRequest, ChatResponse, LanguageModel, MessageRole, Result, Usage,
    last_user_message, validate_base_url, validate_model_id,
};

#[derive(Clone, Debug)]
pub struct OpenAiCompatibleModel {
    model_id: String,
    base_url: Option<String>,
}

impl OpenAiCompatibleModel {
    pub fn new(model_id: impl Into<String>, base_url: Option<String>) -> Result<Self> {
        let model_id = model_id.into();
        validate_model_id(&model_id)?;
        if let Some(base_url) = base_url.as_deref() {
            validate_base_url(base_url)?;
        }
        Ok(Self { model_id, base_url })
    }

    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }
}

impl LanguageModel for OpenAiCompatibleModel {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        let prompt = last_user_message(&request)?;
        Ok(ChatResponse {
            model_id: self.model_id.clone(),
            message: ChatMessage {
                role: MessageRole::Assistant,
                content: format!("openai-compatible:{}:{prompt}", self.model_id),
            },
            usage: Usage {
                input_tokens: request.messages.len() as u32,
                output_tokens: prompt.split_whitespace().count() as u32,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_compatible_adapter_preserves_base_url() {
        let model =
            OpenAiCompatibleModel::new("gpt-compatible", Some("http://localhost:11434".into()))
                .unwrap_or_else(|error| panic!("model config should be valid: {error}"));
        let response = model
            .complete(ChatRequest {
                messages: vec![ChatMessage {
                    role: MessageRole::User,
                    content: "draft".to_string(),
                }],
                max_output_tokens: 64,
            })
            .unwrap_or_else(|error| panic!("model completes: {error}"));

        assert_eq!(model.base_url(), Some("http://localhost:11434"));
        assert!(response.message.content.contains("draft"));
        assert!(OpenAiCompatibleModel::new("", None).is_err());
        assert!(
            OpenAiCompatibleModel::new("gpt-compatible", Some("localhost:11434".into())).is_err()
        );
    }
}
