//! Offline `tdw-llm-anthropic` example: build a `ChatRequest` and complete it
//! through the deterministic sync stub `AnthropicMessagesModel`.
//!
//! This makes NO live API call and needs NO `ANTHROPIC_API_KEY` — the real HTTP
//! client lives behind the `http` feature and is exercised by the env-gated live
//! test, not here.
//!
//! ```text
//! cargo run --example tdw_llm_anthropic_basic -p tdw-llm-anthropic
//! ```

use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};
use tdw_llm_anthropic::AnthropicMessagesModel;

fn main() {
    let model =
        AnthropicMessagesModel::new("claude-haiku-4-5-20251001").expect("model id should be valid");

    let request = ChatRequest {
        messages: vec![
            ChatMessage {
                role: MessageRole::System,
                content: "Reply with exactly one short word.".to_string(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: "Say hi.".to_string(),
            },
        ],
        max_output_tokens: 32,
    };

    let response = model
        .complete(request)
        .expect("offline completion should succeed");

    assert_eq!(response.model_id, "claude-haiku-4-5-20251001");
    assert_eq!(response.message.role, MessageRole::Assistant);
    assert!(response.message.content.contains("Say hi."));
    println!("model:    {}", response.model_id);
    println!("response: {}", response.message.content);
}
