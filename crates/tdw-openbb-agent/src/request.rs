//! `OpenBB` Workspace `QueryRequest` serde types (the request half of the
//! `POST /v1/query` copilot contract).
//!
//! These are tolerant projections of the **public** `OpenBB` Workspace copilot
//! request shape: every struct uses `#[serde(default)]` on optional fields and
//! ignores unknown keys (no `deny_unknown_fields`), so a Workspace payload that
//! carries fields this bridge does not yet model still deserializes. Field
//! names match the documented `openbb-ai` `QueryRequest` JSON (`snake_case`),
//! cited in [`crate`].
//!
//! Doc source: <https://docs.openbb.co/workspace/copilots> (the `QueryRequest`
//! schema — `messages`, `widgets.primary` / `widgets.secondary`, `urls`).

use serde::{Deserialize, Serialize};

/// The role of a conversation message in a Workspace copilot exchange.
///
/// Workspace uses `"human"` / `"ai"` (not `"user"` / `"assistant"`) and `"tool"`
/// for a folded widget-data result. An unrecognized role deserializes to
/// [`MessageRole::Unknown`] so a future role never fails the whole request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    /// A message authored by the end user.
    #[serde(rename = "human")]
    Human,
    /// A message authored by the copilot (a prior assistant turn).
    #[serde(rename = "ai")]
    Ai,
    /// A folded tool result — the data the frontend fetched for a
    /// previously-emitted `get_widget_data` function call.
    #[serde(rename = "tool")]
    Tool,
    /// Any role the bridge does not model; preserved so the request still
    /// deserializes.
    #[serde(other)]
    Unknown,
}

/// One message in the `messages` array of a [`QueryRequest`].
///
/// `content` is tolerant: Workspace sends a plain string for human/ai turns but
/// a structured object for a `tool` turn (the function-call result). Keeping it
/// as a raw [`serde_json::Value`] lets both shapes round-trip without losing
/// data; [`Message::text`] extracts a best-effort string for prompt assembly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Who authored the message.
    pub role: MessageRole,
    /// The message body: a string for human/ai turns, an object/array for a
    /// folded `tool` result.
    #[serde(default)]
    pub content: serde_json::Value,
}

impl Message {
    /// Best-effort plain-text rendering of [`Message::content`] for prompt
    /// assembly: a JSON string is returned verbatim; any other JSON value is
    /// rendered as compact JSON (so a structured `tool` result is still
    /// readable in the model context).
    #[must_use]
    pub fn text(&self) -> String {
        match &self.content {
            serde_json::Value::String(text) => text.clone(),
            serde_json::Value::Null => String::new(),
            other => other.to_string(),
        }
    }
}

/// One widget the user has on the dashboard, as described in the request's
/// `widgets.primary` / `widgets.secondary` arrays.
///
/// Workspace identifies a widget by a UUID (`uuid` in newer payloads,
/// `widget_id` in older ones) and carries its display `name`, a `description`,
/// the `origin`/`widget_id` data-endpoint binding, and the `params` (current
/// parameter values). Only the fields the bridge needs are modeled; everything
/// else is ignored tolerantly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Widget {
    /// The widget instance UUID (newer Workspace payloads).
    #[serde(default)]
    pub uuid: Option<String>,
    /// The widget definition id (older payloads / the `widgets.json` key).
    #[serde(default)]
    pub widget_id: Option<String>,
    /// Human-readable widget name shown on the dashboard.
    #[serde(default)]
    pub name: Option<String>,
    /// Longer description of what the widget shows.
    #[serde(default)]
    pub description: Option<String>,
    /// The widget's current parameter values (the editable controls).
    #[serde(default)]
    pub params: serde_json::Value,
}

impl Widget {
    /// The stable identifier for this widget, preferring `uuid` and falling back
    /// to `widget_id`. Returns `None` when neither is present.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.uuid
            .as_deref()
            .or(self.widget_id.as_deref())
            .filter(|id| !id.is_empty())
    }

    /// A short human label for the widget (its `name`, else its id).
    #[must_use]
    pub fn label(&self) -> &str {
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .or_else(|| self.id())
            .unwrap_or("widget")
    }
}

/// The `widgets` block of a [`QueryRequest`]: the primary (explicitly added)
/// widgets and the secondary (context) widgets on the dashboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Widgets {
    /// The primary widgets — the ones the user explicitly attached to the chat.
    #[serde(default)]
    pub primary: Vec<Widget>,
    /// The secondary widgets — additional dashboard context.
    #[serde(default)]
    pub secondary: Vec<Widget>,
}

/// A Workspace copilot `POST /v1/query` request body.
///
/// Tolerant by construction: optional fields default and unknown fields are
/// ignored, so a richer Workspace payload (extra metadata, settings, etc.)
/// still deserializes into the subset the bridge consumes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QueryRequest {
    /// The conversation so far (human/ai turns plus any folded `tool` results).
    #[serde(default)]
    pub messages: Vec<Message>,
    /// The widgets on the dashboard, split into primary and secondary.
    #[serde(default)]
    pub widgets: Widgets,
    /// Any URLs the user attached for context.
    #[serde(default)]
    pub urls: Vec<String>,
    /// The caller's IANA timezone, when supplied.
    #[serde(default)]
    pub timezone: Option<String>,
}

impl QueryRequest {
    /// The most recent `human` message's text, if any — the user's current
    /// question.
    #[must_use]
    pub fn last_human_text(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Human)
            .map(Message::text)
    }

    /// Whether the conversation already carries a folded `tool` result (the
    /// second leg of the stateless two-request widget-data pattern).
    #[must_use]
    pub fn has_tool_result(&self) -> bool {
        self.messages
            .iter()
            .any(|message| message.role == MessageRole::Tool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_doc_fixture_with_human_message_and_primary_widget() {
        let raw = r#"{
            "messages": [{"role": "human", "content": "How did AAPL do?"}],
            "widgets": {
                "primary": [{
                    "uuid": "w-1",
                    "name": "Price History",
                    "description": "AAPL daily close",
                    "params": {"symbol": "AAPL"}
                }],
                "secondary": []
            },
            "urls": ["https://example.com"],
            "timezone": "America/New_York"
        }"#;
        let request: QueryRequest = serde_json::from_str(raw).expect("parses");
        assert_eq!(request.messages.len(), 1);
        assert_eq!(request.messages[0].role, MessageRole::Human);
        assert_eq!(
            request.last_human_text().as_deref(),
            Some("How did AAPL do?")
        );
        assert_eq!(request.widgets.primary.len(), 1);
        assert_eq!(request.widgets.primary[0].id(), Some("w-1"));
        assert_eq!(request.widgets.primary[0].label(), "Price History");
        assert_eq!(request.timezone.as_deref(), Some("America/New_York"));
        assert!(!request.has_tool_result());
    }

    #[test]
    fn ignores_unknown_fields_tolerantly() {
        let raw = r#"{
            "messages": [],
            "unknown_top_level": 42,
            "widgets": {"primary": [{"widget_id": "x", "future_field": true}]}
        }"#;
        let request: QueryRequest = serde_json::from_str(raw).expect("tolerant parse");
        assert_eq!(request.widgets.primary[0].id(), Some("x"));
    }

    #[test]
    fn unknown_role_does_not_fail_parse() {
        let raw = r#"{"messages": [{"role": "system", "content": "hi"}]}"#;
        let request: QueryRequest = serde_json::from_str(raw).expect("parses");
        assert_eq!(request.messages[0].role, MessageRole::Unknown);
    }

    #[test]
    fn tool_message_with_structured_content_renders_as_json_text() {
        let raw = r#"{"messages": [
            {"role": "human", "content": "q"},
            {"role": "tool", "content": {"rows": [{"close": 1}]}}
        ]}"#;
        let request: QueryRequest = serde_json::from_str(raw).expect("parses");
        assert!(request.has_tool_result());
        let tool = &request.messages[1];
        assert_eq!(tool.role, MessageRole::Tool);
        assert!(tool.text().contains("close"));
    }

    #[test]
    fn widget_id_prefers_uuid_then_widget_id() {
        let both = Widget {
            uuid: Some("u".to_string()),
            widget_id: Some("w".to_string()),
            ..Widget::default()
        };
        assert_eq!(both.id(), Some("u"));
        let only_widget_id = Widget {
            widget_id: Some("w".to_string()),
            ..Widget::default()
        };
        assert_eq!(only_widget_id.id(), Some("w"));
    }

    #[test]
    fn message_text_round_trips_string_and_null() {
        let string_message = Message {
            role: MessageRole::Ai,
            content: serde_json::Value::String("hello".to_string()),
        };
        assert_eq!(string_message.text(), "hello");
        let null_message = Message {
            role: MessageRole::Ai,
            content: serde_json::Value::Null,
        };
        assert_eq!(null_message.text(), "");
    }
}
