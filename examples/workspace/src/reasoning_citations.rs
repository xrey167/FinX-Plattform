//! Example 50 — reasoning steps + citations to a dashboard widget.
//!
//! Beyond streaming text, a copilot narrates its work with `reasoning_step`
//! events and attributes its answer to the widgets it used with a closing
//! `citations` event. This example drives the *pure* copilot sequencer
//! ([`tdw_openbb_agent::answer`]) in-process with the offline `StubLanguageModel`
//! on a conversation that already carries a folded widget-data result, so the
//! second-leg shape — opening reasoning step, streamed `message_chunk`s, a
//! SUCCESS reasoning step, then `citations` pointing at the source widget — is
//! produced with no server and no network.
//!
//! [`answered_turn`] returns the ordered [`SseEvent`]s; [`sse_transcript`]
//! renders them to the exact wire frames a transport would write, for a golden
//! assertion.

use serde_json::json;
use tdw_eval_runner::StubLanguageModel;
use tdw_openbb_agent::{Message, MessageRole, QueryRequest, SseEvent, Widget, Widgets, answer};

/// The widget uuid the example's conversation cites.
pub const WIDGET_UUID: &str = "w-aapl-history";

/// Build the second-leg request: a question about a primary equity widget, with
/// the fetched rows already folded in as a `tool` message.
#[must_use]
pub fn request() -> QueryRequest {
    QueryRequest {
        messages: vec![
            Message {
                role: MessageRole::Human,
                content: json!("How did AAPL close this week?"),
            },
            Message {
                role: MessageRole::Tool,
                content: json!({
                    "results": [
                        { "date": "2024-01-04", "close": 183.95 },
                        { "date": "2024-01-05", "close": 185.60 }
                    ]
                }),
            },
        ],
        widgets: Widgets {
            primary: vec![Widget {
                uuid: Some(WIDGET_UUID.to_string()),
                name: Some("AAPL Price History".to_string()),
                params: json!({ "symbol": "AAPL" }),
                ..Widget::default()
            }],
            secondary: vec![],
        },
        ..QueryRequest::default()
    }
}

/// Drive the pure sequencer over the offline stub and return the ordered events.
#[must_use]
pub fn answered_turn() -> Vec<SseEvent> {
    answer(&request(), &StubLanguageModel).events
}

/// Render the answered turn to the exact SSE wire frames a transport would write.
#[must_use]
pub fn sse_transcript() -> String {
    answered_turn().iter().map(SseEvent::to_sse_frame).collect()
}
