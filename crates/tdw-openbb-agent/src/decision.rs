//! The widget-data decision: whether the agent should ask the frontend to fetch
//! a primary widget's data before answering (the stateless two-request
//! pattern).
//!
//! `OpenBB` Workspace copilots cannot read widget *data* directly from the
//! request — the request carries only the widget *descriptions* (name,
//! description, current params). When the user's question references a primary
//! widget whose data is not yet folded in, the agent emits a `get_widget_data`
//! event naming that widget and closes the stream; the frontend re-POSTs the
//! whole conversation with a `tool` message carrying the fetched data, and the
//! agent answers on that second leg.
//!
//! [`needs_widget_data`] is the pure decision function: it returns the
//! [`WidgetDataRequest`] to emit, or `None` when the agent can answer directly
//! (no primary widgets, or a `tool` result is already present).

use crate::event::WidgetDataRequest;
use crate::request::QueryRequest;

/// Decide whether the agent should request a primary widget's data before
/// answering.
///
/// Returns `Some(request)` naming the first primary widget to fetch when **all**
/// of the following hold:
/// - the conversation does not already carry a folded `tool` result (this is
///   the first leg of the two-request pattern), and
/// - there is at least one primary widget with a resolvable id.
///
/// Returns `None` (answer directly) when a `tool` result is already present
/// (second leg) or there are no primary widgets to fetch.
///
/// The emitted request carries the widget's current `params` as
/// `input_arguments` when they are a non-empty object, so the frontend fetches
/// with the same parameters the user has set on the widget.
#[must_use]
pub fn needs_widget_data(request: &QueryRequest) -> Option<WidgetDataRequest> {
    if request.has_tool_result() {
        return None;
    }
    let widget = request
        .widgets
        .primary
        .iter()
        .find(|widget| widget.id().is_some())?;
    let id = widget.id()?.to_string();
    let arguments = match &widget.params {
        serde_json::Value::Object(map) if !map.is_empty() => {
            Some(serde_json::Value::Object(map.clone()))
        }
        _ => None,
    };
    Some(match arguments {
        Some(arguments) => WidgetDataRequest::with_arguments(id, arguments),
        None => WidgetDataRequest::new(id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Message, MessageRole, Widget, Widgets};
    use serde_json::json;

    fn human(text: &str) -> Message {
        Message {
            role: MessageRole::Human,
            content: json!(text),
        }
    }

    #[test]
    fn requests_first_primary_widget_with_params_as_arguments() {
        let request = QueryRequest {
            messages: vec![human("How did AAPL do?")],
            widgets: Widgets {
                primary: vec![Widget {
                    uuid: Some("w-1".to_string()),
                    params: json!({"symbol": "AAPL"}),
                    ..Widget::default()
                }],
                secondary: vec![],
            },
            ..QueryRequest::default()
        };
        let decision = needs_widget_data(&request).expect("needs data");
        assert_eq!(decision.widget_id, "w-1");
        assert_eq!(decision.input_arguments, Some(json!({"symbol": "AAPL"})));
    }

    #[test]
    fn no_request_when_tool_result_already_present() {
        let request = QueryRequest {
            messages: vec![
                human("q"),
                Message {
                    role: MessageRole::Tool,
                    content: json!({"rows": []}),
                },
            ],
            widgets: Widgets {
                primary: vec![Widget {
                    uuid: Some("w-1".to_string()),
                    ..Widget::default()
                }],
                secondary: vec![],
            },
            ..QueryRequest::default()
        };
        assert!(needs_widget_data(&request).is_none());
    }

    #[test]
    fn no_request_when_no_primary_widgets() {
        let request = QueryRequest {
            messages: vec![human("hello")],
            ..QueryRequest::default()
        };
        assert!(needs_widget_data(&request).is_none());
    }

    #[test]
    fn empty_params_yield_no_input_arguments() {
        let request = QueryRequest {
            messages: vec![human("q")],
            widgets: Widgets {
                primary: vec![Widget {
                    widget_id: Some("w-2".to_string()),
                    params: json!({}),
                    ..Widget::default()
                }],
                secondary: vec![],
            },
            ..QueryRequest::default()
        };
        let decision = needs_widget_data(&request).expect("needs data");
        assert_eq!(decision.widget_id, "w-2");
        assert_eq!(decision.input_arguments, None);
    }

    #[test]
    fn skips_primary_widget_without_id() {
        let request = QueryRequest {
            messages: vec![human("q")],
            widgets: Widgets {
                primary: vec![
                    Widget::default(),
                    Widget {
                        uuid: Some("w-3".to_string()),
                        ..Widget::default()
                    },
                ],
                secondary: vec![],
            },
            ..QueryRequest::default()
        };
        let decision = needs_widget_data(&request).expect("needs data");
        assert_eq!(decision.widget_id, "w-3");
    }
}
