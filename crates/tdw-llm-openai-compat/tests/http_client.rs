//! Live integration test for the OpenAI-compatible Chat Completions
//! HTTP client.
//!
//! Doubly gated:
//!   - Compiled only with `--features http` (no reqwest dep otherwise).
//!   - Runs only when `TDW_OPENAI_COMPAT_LIVE=1` and an API key env
//!     var is set. Parser/body cassette tests live inside the module.

#![cfg(feature = "http")]

use tdw_llm::{ChatMessage, ChatRequest, MessageRole};
use tdw_llm_openai_compat::OpenAiCompatibleHttpClient;

const API_KEY_ENVS: [&str; 2] = ["TDW_OPENAI_COMPAT_API_KEY", "OPENAI_API_KEY"];

fn live_enabled() -> bool {
    std::env::var("TDW_OPENAI_COMPAT_LIVE").ok().as_deref() == Some("1")
}

fn api_key() -> Option<String> {
    API_KEY_ENVS.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn base_url() -> Option<String> {
    std::env::var("TDW_OPENAI_COMPAT_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn model_id() -> String {
    std::env::var("TDW_OPENAI_COMPAT_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "gpt-4o-mini".to_string())
}

#[tokio::test]
async fn live_openai_compatible_returns_completion_when_env_var_set() {
    if !live_enabled() {
        eprintln!("TDW_OPENAI_COMPAT_LIVE != 1; skipping live openai-compatible test");
        return;
    }
    let Some(key) = api_key() else {
        eprintln!("neither TDW_OPENAI_COMPAT_API_KEY nor OPENAI_API_KEY set; skipping live test");
        return;
    };

    let mut client = OpenAiCompatibleHttpClient::new(key, model_id())
        .unwrap_or_else(|error| panic!("client must build: {error}"));
    if let Some(url) = base_url() {
        client = client
            .with_base_url(url)
            .unwrap_or_else(|error| panic!("base url must parse: {error}"));
    }

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
    assert_eq!(response.message.role, MessageRole::Assistant);
    assert_eq!(response.model_id, client.model_id());
}
