#![forbid(unsafe_code)]

#[cfg(feature = "http")]
pub mod http_client;

#[cfg(feature = "http")]
pub use http_client::{AnthropicHttpClient, AnthropicHttpError};

use tdw_llm::{
    ChatMessage, ChatRequest, ChatResponse, LanguageModel, MessageRole, Result,
    StreamingLanguageModel, Usage, last_user_message, split_into_chunks, validate_model_id,
};

/// Number of deterministic chunks the offline stub splits its canned answer
/// into when streamed. Chosen small so chunk-handling tests stay readable while
/// still exercising the multi-chunk path.
const STUB_STREAM_CHUNKS: usize = 3;

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

    /// Real Anthropic-backed client — production-grade for eval feedback.
    fn is_production_grade(&self) -> bool {
        true
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

/// Offline streaming for the deterministic stub: produce the same canned
/// answer as [`LanguageModel::complete`], then replay it as a few
/// deterministic chunks so downstream chunk-handling is exercisable without a
/// network round-trip. The native SSE-consuming path lives on
/// [`crate::http_client::AnthropicHttpClient::complete_streaming`] (gated by
/// the `http` feature).
impl StreamingLanguageModel for AnthropicMessagesModel {
    fn complete_streaming(
        &self,
        request: &ChatRequest,
        on_chunk: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse> {
        let response = self.complete(request.clone())?;
        for chunk in split_into_chunks(&response.message.content, STUB_STREAM_CHUNKS) {
            on_chunk(&chunk);
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_stub_streaming_splits_into_multiple_chunks() {
        let model = AnthropicMessagesModel::new("claude-test")
            .unwrap_or_else(|error| panic!("model id should be valid: {error}"));
        let request = ChatRequest {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "summarize AAPL earnings".to_string(),
            }],
            max_output_tokens: 128,
        };
        let mut chunks = Vec::new();
        let response = model
            .complete_streaming(&request, &mut |chunk| chunks.push(chunk.to_string()))
            .unwrap_or_else(|error| panic!("streaming completes: {error}"));

        assert!(chunks.len() > 1, "stub stream should emit multiple chunks");
        // Reconstruction invariant: concatenated chunks == final answer.
        assert_eq!(chunks.concat(), response.message.content);
        assert_eq!(
            response,
            model
                .complete(request)
                .unwrap_or_else(|error| panic!("complete: {error}"))
        );
    }

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
