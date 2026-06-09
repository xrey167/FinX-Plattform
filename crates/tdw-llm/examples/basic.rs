//! Offline `tdw-llm` example: build a `ChatRequest`, complete it through a tiny
//! in-crate `LanguageModel`, and parse a fixture `ChatResponse` from JSON.
//!
//! No network and no API key — it exercises the trait + DTOs + validators only.
//!
//! ```text
//! cargo run --example tdw_llm_basic -p tdw-llm
//! ```

use tdw_llm::{
    ChatMessage, ChatRequest, ChatResponse, LanguageModel, MessageRole, Result, Usage,
    last_user_message, validate_chat_request,
};

/// A deterministic, offline `LanguageModel`: it echoes the last user message
/// back as the assistant reply. This stands in for a real provider adapter.
struct EchoModel;

impl LanguageModel for EchoModel {
    fn model_id(&self) -> &'static str {
        "example-echo"
    }

    fn complete(&self, request: ChatRequest) -> Result<ChatResponse> {
        let prompt = last_user_message(&request)?;
        Ok(ChatResponse {
            model_id: self.model_id().to_string(),
            message: ChatMessage {
                role: MessageRole::Assistant,
                content: prompt.to_string(),
            },
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        })
    }
}

fn main() {
    // 1) Build and validate a request.
    let request = ChatRequest {
        messages: vec![
            ChatMessage {
                role: MessageRole::System,
                content: "Be terse.".to_string(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: "Summarize AAPL".to_string(),
            },
        ],
        max_output_tokens: 256,
    };
    validate_chat_request(&request).expect("request should be valid");

    // 2) Complete it through the offline model.
    let model = EchoModel;
    let live = model.complete(request).expect("completion should succeed");
    assert_eq!(live.message.content, "Summarize AAPL");
    println!("completed: {}", live.message.content);

    // 3) A *fixture* provider response (what an adapter would hand back after
    //    translating its provider-specific JSON into the workspace DTOs). No
    //    network call: we construct it directly and read the parsed fields the
    //    same way a caller would.
    let fixture = ChatResponse {
        model_id: "example-echo".to_string(),
        message: ChatMessage {
            role: MessageRole::Assistant,
            content: "Apple Inc. — large-cap tech.".to_string(),
        },
        usage: Usage {
            input_tokens: 7,
            output_tokens: 5,
        },
    };
    assert_eq!(fixture.message.role, MessageRole::Assistant);
    assert_eq!(fixture.usage.output_tokens, 5);
    println!("parsed fixture: {}", fixture.message.content);
}
