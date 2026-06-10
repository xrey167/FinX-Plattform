//! Prompt assembly: turn a Workspace [`QueryRequest`] into a [`ChatRequest`] for
//! the `tdw-llm` [`StreamingLanguageModel`](tdw_llm::StreamingLanguageModel).
//!
//! The assembly is pure and deterministic (no I/O): it threads a system primer,
//! a rendered description of the dashboard widgets, the conversation history,
//! and any folded `tool` (widget-data) results into the message list the model
//! consumes.
//!
//! Role mapping (Workspace → `tdw-llm`): `human → User`, `ai → Assistant`,
//! `tool → Tool`. The widget descriptions and system primer are prepended as a
//! single `System` message so the model knows what is on the dashboard and what
//! it may ask the frontend to fetch.

use std::fmt::Write as _;

use tdw_llm::{ChatMessage, ChatRequest, MessageRole as LlmRole};

use crate::request::{MessageRole, QueryRequest, Widget};

/// The default per-answer output-token budget for an assembled request.
///
/// Conservative; the handler may override before calling the model.
pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 1024;

/// The system primer prepended to every assembled prompt. Describes the agent's
/// role and the stateless widget-data fetch protocol so the model's answers stay
/// grounded in the dashboard.
const SYSTEM_PRIMER: &str = "You are a financial analysis copilot embedded in a dashboard. \
Answer the user's question using the dashboard widgets described below and any widget data \
provided as tool results. When you use a widget's data, attribute it. Be concise and factual; \
do not fabricate values you were not given.";

/// Render a single widget as a one-line description for the system context.
fn render_widget(widget: &Widget) -> String {
    let mut line = format!("- {}", widget.label());
    if let Some(id) = widget.id() {
        let _ = write!(line, " (id: {id})");
    }
    if let Some(description) = widget.description.as_deref().filter(|d| !d.is_empty()) {
        let _ = write!(line, ": {description}");
    }
    if let serde_json::Value::Object(map) = &widget.params
        && !map.is_empty()
    {
        let params = serde_json::to_string(&widget.params).unwrap_or_else(|_| "{}".to_string());
        let _ = write!(line, " [params: {params}]");
    }
    line
}

/// Build the system message text: the primer plus a rendered list of the
/// primary and secondary widgets.
fn system_text(request: &QueryRequest) -> String {
    let mut text = String::from(SYSTEM_PRIMER);
    if !request.widgets.primary.is_empty() {
        text.push_str("\n\nPrimary widgets:\n");
        for widget in &request.widgets.primary {
            text.push_str(&render_widget(widget));
            text.push('\n');
        }
    }
    if !request.widgets.secondary.is_empty() {
        text.push_str("\nSecondary widgets:\n");
        for widget in &request.widgets.secondary {
            text.push_str(&render_widget(widget));
            text.push('\n');
        }
    }
    if !request.urls.is_empty() {
        text.push_str("\nAttached URLs:\n");
        for url in &request.urls {
            let _ = writeln!(text, "- {url}");
        }
    }
    text
}

/// Map a Workspace message role to a `tdw-llm` role. An `Unknown` role is
/// folded to `User` so its content still reaches the model.
const fn map_role(role: MessageRole) -> LlmRole {
    match role {
        MessageRole::Human | MessageRole::Unknown => LlmRole::User,
        MessageRole::Ai => LlmRole::Assistant,
        MessageRole::Tool => LlmRole::Tool,
    }
}

/// Assemble a [`ChatRequest`] from `request` with the given output-token budget.
///
/// The message list is: one `System` message (primer + widget descriptions),
/// then every conversation message mapped to its `tdw-llm` role. A `tool`
/// message is rendered with a `Widget data:` prefix so the model can tell a
/// folded widget-data result apart from prose. Empty-content messages are
/// dropped so the model never receives an empty turn (`tdw-llm` rejects empty
/// content).
#[must_use]
pub fn assemble_chat_request(request: &QueryRequest, max_output_tokens: u32) -> ChatRequest {
    let mut messages = vec![ChatMessage {
        role: LlmRole::System,
        content: system_text(request),
    }];
    for message in &request.messages {
        let text = message.text();
        if text.is_empty() {
            continue;
        }
        let content = if message.role == MessageRole::Tool {
            format!("Widget data: {text}")
        } else {
            text
        };
        messages.push(ChatMessage {
            role: map_role(message.role),
            content,
        });
    }
    ChatRequest {
        messages,
        max_output_tokens,
    }
}

/// Assemble a [`ChatRequest`] with the [`DEFAULT_MAX_OUTPUT_TOKENS`] budget.
#[must_use]
pub fn assemble_default(request: &QueryRequest) -> ChatRequest {
    assemble_chat_request(request, DEFAULT_MAX_OUTPUT_TOKENS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Message, Widget, Widgets};
    use serde_json::json;

    fn human(text: &str) -> Message {
        Message {
            role: MessageRole::Human,
            content: json!(text),
        }
    }

    #[test]
    fn assembles_system_message_plus_human_turn() {
        let request = QueryRequest {
            messages: vec![human("How did AAPL do?")],
            widgets: Widgets {
                primary: vec![Widget {
                    uuid: Some("w-1".to_string()),
                    name: Some("Price History".to_string()),
                    description: Some("AAPL daily close".to_string()),
                    params: json!({"symbol": "AAPL"}),
                    ..Widget::default()
                }],
                secondary: vec![],
            },
            ..QueryRequest::default()
        };
        let chat = assemble_default(&request);
        assert_eq!(chat.messages[0].role, LlmRole::System);
        assert!(chat.messages[0].content.contains("Price History"));
        assert!(chat.messages[0].content.contains("id: w-1"));
        assert!(chat.messages[0].content.contains("symbol"));
        assert_eq!(chat.messages[1].role, LlmRole::User);
        assert_eq!(chat.messages[1].content, "How did AAPL do?");
        assert_eq!(chat.max_output_tokens, DEFAULT_MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn folds_tool_result_with_prefix() {
        let request = QueryRequest {
            messages: vec![
                human("How did AAPL do?"),
                Message {
                    role: MessageRole::Tool,
                    content: json!({"rows": [{"close": 190.0}]}),
                },
            ],
            ..QueryRequest::default()
        };
        let chat = assemble_default(&request);
        // System, User, Tool.
        assert_eq!(chat.messages.len(), 3);
        assert_eq!(chat.messages[2].role, LlmRole::Tool);
        assert!(chat.messages[2].content.starts_with("Widget data: "));
        assert!(chat.messages[2].content.contains("190"));
    }

    #[test]
    fn maps_ai_role_to_assistant() {
        let request = QueryRequest {
            messages: vec![
                human("hi"),
                Message {
                    role: MessageRole::Ai,
                    content: json!("hello"),
                },
            ],
            ..QueryRequest::default()
        };
        let chat = assemble_default(&request);
        assert_eq!(chat.messages[2].role, LlmRole::Assistant);
        assert_eq!(chat.messages[2].content, "hello");
    }

    #[test]
    fn drops_empty_content_turns() {
        let request = QueryRequest {
            messages: vec![human(""), human("real question")],
            ..QueryRequest::default()
        };
        let chat = assemble_default(&request);
        // System + one non-empty user turn.
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].content, "real question");
    }

    #[test]
    fn custom_token_budget_is_threaded() {
        let request = QueryRequest {
            messages: vec![human("q")],
            ..QueryRequest::default()
        };
        let chat = assemble_chat_request(&request, 256);
        assert_eq!(chat.max_output_tokens, 256);
    }
}
