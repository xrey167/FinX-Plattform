//! Offline `tdw-llm-openai-compat` example: build a `ChatRequest` and complete
//! it through the deterministic sync stub `OpenAiCompatibleModel`, targeting a
//! local gateway base URL.
//!
//! NO live API call and NO API key — the real HTTP client lives behind the
//! `http` feature and is exercised by the env-gated live test, not here.
//!
//! ```text
//! cargo run --example tdw_llm_openai_compat_basic -p tdw-llm-openai-compat
//! ```

use tdw_llm::{ChatMessage, ChatRequest, LanguageModel, MessageRole};
use tdw_llm_openai_compat::OpenAiCompatibleModel;

fn main() {
    // The optional base URL points at a self-hosted OpenAI-compatible gateway
    // (e.g. Ollama). It is validated on construction.
    let model = OpenAiCompatibleModel::new("gpt-compatible", Some("http://localhost:11434".into()))
        .expect("model config should be valid");
    assert_eq!(model.base_url(), Some("http://localhost:11434"));

    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: "draft a one-line AAPL note".to_string(),
        }],
        max_output_tokens: 64,
    };

    let response = model
        .complete(request)
        .expect("offline completion should succeed");

    assert_eq!(response.message.role, MessageRole::Assistant);
    assert!(
        response
            .message
            .content
            .contains("draft a one-line AAPL note")
    );
    println!("base_url: {:?}", model.base_url());
    println!("response: {}", response.message.content);
}
