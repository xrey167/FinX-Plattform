//! Live integration test for the Anthropic Messages HTTP client.
//!
//! Doubly gated:
//!   - Compiled only with `--features http` (no reqwest dep otherwise).
//!   - Runs only when `TDW_ANTHROPIC_LIVE=1` AND `ANTHROPIC_API_KEY`
//!     are set in the environment. Cassette + role-translation tests
//!     for the parser/builder live as unit tests inside the module
//!     (they need pub(crate) access).

#![cfg(feature = "http")]

use tdw_llm::{ChatMessage, ChatRequest, MessageRole};
use tdw_llm_anthropic::AnthropicHttpClient;

fn live_enabled() -> bool {
    std::env::var("TDW_ANTHROPIC_LIVE").ok().as_deref() == Some("1")
}

fn api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY").ok()
}

#[tokio::test]
async fn live_anthropic_returns_completion_when_env_var_set() {
    if !live_enabled() {
        eprintln!("TDW_ANTHROPIC_LIVE != 1; skipping live anthropic integration test");
        return;
    }
    let Some(key) = api_key() else {
        eprintln!("ANTHROPIC_API_KEY not set; skipping live anthropic integration test");
        return;
    };

    let client = AnthropicHttpClient::new(key, "claude-haiku-4-5-20251001")
        .unwrap_or_else(|error| panic!("client must build: {error}"));

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

    let response = client
        .complete(request)
        .await
        .unwrap_or_else(|error| panic!("live request must succeed: {error}"));

    assert!(
        !response.message.content.trim().is_empty(),
        "live response content must not be empty: {response:?}"
    );
    assert!(
        response.usage.output_tokens > 0,
        "live response must report nonzero output_tokens: {response:?}"
    );
    assert_eq!(response.model_id, "claude-haiku-4-5-20251001");
    assert_eq!(response.message.role, MessageRole::Assistant);
}
