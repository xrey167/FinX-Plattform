//! The widget-data decision: which widgets the agent fetches before answering.
//!
//! This generalizes the stateless two-request pattern to multiple widgets,
//! secondary widgets, and chained multi-round fetches.
//!
//! `OpenBB` Workspace copilots cannot read widget *data* directly from the
//! request — the request carries only the widget *descriptions* (name,
//! description, current params). When the user's question references widgets
//! whose data is not yet folded in, the agent emits a `get_widget_data` event
//! naming them and closes the stream; the frontend re-POSTs the whole
//! conversation with `tool` messages carrying the fetched data, and the agent
//! answers (or asks for the next batch) on the next leg.
//!
//! # Multi-widget, secondary, and multi-round
//!
//! [`widget_data_requests`] is the general decision: it returns **every**
//! fetchable widget not yet satisfied this round — all primary widgets, plus
//! the secondary (dashboard-context) widgets when `widget-dashboard-search` is
//! enabled. It supports **chained multi-round** fetches by counting the folded
//! `tool` results already present and skipping that many widgets, so a frontend
//! that folds results one (or a few) at a time keeps being asked for the rest
//! until the whole set is satisfied. When nothing remains to fetch it returns an
//! empty vec (answer directly).
//!
//! [`needs_widget_data`] is the original single-widget convenience kept for
//! callers that only want the first fetch.

use crate::event::WidgetDataRequest;
use crate::request::{QueryRequest, Widget};

/// Build the [`WidgetDataRequest`] for `widget`, carrying its current non-empty
/// object `params` as `input_arguments`.
fn request_for(widget: &Widget) -> Option<WidgetDataRequest> {
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

/// The ordered list of fetchable widgets for `request`: primary first, then —
/// when `include_secondary` is set (the `widget-dashboard-search` feature) —
/// the secondary dashboard-context widgets. Widgets without a resolvable id are
/// dropped.
fn fetchable(request: &QueryRequest, include_secondary: bool) -> Vec<&Widget> {
    let primary = request.widgets.primary.iter();
    let secondary = include_secondary
        .then(|| request.widgets.secondary.iter())
        .into_iter()
        .flatten();
    primary
        .chain(secondary)
        .filter(|widget| widget.id().is_some())
        .collect()
}

/// Decide which widgets the agent should ask the frontend to fetch this round.
///
/// Returns the batch of [`WidgetDataRequest`]s for every fetchable widget not
/// yet satisfied (see the module docs). `include_secondary` wires the
/// `widget-dashboard-search` feature: when `true`, secondary (dashboard-context)
/// widgets are fetched after the primary ones.
///
/// Multi-round: the number of folded `tool` results already in the conversation
/// is treated as the count of widgets already fetched; that many leading widgets
/// are skipped and the remainder requested. An empty result means every widget
/// is satisfied and the agent should answer.
#[must_use]
pub fn widget_data_requests(
    request: &QueryRequest,
    include_secondary: bool,
) -> Vec<WidgetDataRequest> {
    let already_fetched = request
        .messages
        .iter()
        .filter(|message| message.role == crate::request::MessageRole::Tool)
        .count();
    fetchable(request, include_secondary)
        .into_iter()
        .skip(already_fetched)
        .filter_map(request_for)
        .collect()
}

/// Decide whether the agent should request the **first** primary widget's data
/// before answering (the original single-widget convenience).
///
/// Returns `Some(request)` for the first primary widget when no `tool` result is
/// folded in yet; `None` when a `tool` result is present (second leg) or there
/// are no primary widgets. Prefer [`widget_data_requests`] for the multi-widget
/// / secondary / multi-round path.
#[must_use]
pub fn needs_widget_data(request: &QueryRequest) -> Option<WidgetDataRequest> {
    if request.has_tool_result() {
        return None;
    }
    request
        .widgets
        .primary
        .iter()
        .find(|widget| widget.id().is_some())
        .and_then(request_for)
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

    fn widget(id: &str, params: serde_json::Value) -> Widget {
        Widget {
            uuid: Some(id.to_string()),
            params,
            ..Widget::default()
        }
    }

    #[test]
    fn requests_all_primary_widgets_in_one_batch() {
        let request = QueryRequest {
            messages: vec![human("compare AAPL and MSFT")],
            widgets: Widgets {
                primary: vec![
                    widget("w-1", json!({"symbol": "AAPL"})),
                    widget("w-2", json!({"symbol": "MSFT"})),
                ],
                secondary: vec![],
            },
            ..QueryRequest::default()
        };
        let batch = widget_data_requests(&request, false);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].widget_id, "w-1");
        assert_eq!(batch[1].widget_id, "w-2");
        assert_eq!(batch[1].input_arguments, Some(json!({"symbol": "MSFT"})));
    }

    #[test]
    fn includes_secondary_widgets_only_when_search_enabled() {
        let request = QueryRequest {
            messages: vec![human("q")],
            widgets: Widgets {
                primary: vec![widget("w-1", json!({}))],
                secondary: vec![widget("w-ctx", json!({}))],
            },
            ..QueryRequest::default()
        };
        // Without search: primary only.
        let without = widget_data_requests(&request, false);
        assert_eq!(without.len(), 1);
        assert_eq!(without[0].widget_id, "w-1");
        // With search: primary + secondary.
        let with = widget_data_requests(&request, true);
        assert_eq!(with.len(), 2);
        assert_eq!(with[1].widget_id, "w-ctx");
    }

    #[test]
    fn multi_round_skips_already_fetched_widgets() {
        // Two primary widgets; one tool result folded in => the second leg
        // requests only the remaining (second) widget.
        let request = QueryRequest {
            messages: vec![
                human("compare"),
                Message {
                    role: MessageRole::Tool,
                    content: json!({"rows": []}),
                },
            ],
            widgets: Widgets {
                primary: vec![widget("w-1", json!({})), widget("w-2", json!({}))],
                secondary: vec![],
            },
            ..QueryRequest::default()
        };
        let batch = widget_data_requests(&request, false);
        assert_eq!(batch.len(), 1, "first widget already fetched");
        assert_eq!(batch[0].widget_id, "w-2");
    }

    #[test]
    fn no_requests_when_all_widgets_satisfied() {
        let request = QueryRequest {
            messages: vec![
                human("q"),
                Message {
                    role: MessageRole::Tool,
                    content: json!({"rows": []}),
                },
            ],
            widgets: Widgets {
                primary: vec![widget("w-1", json!({}))],
                secondary: vec![],
            },
            ..QueryRequest::default()
        };
        assert!(widget_data_requests(&request, false).is_empty());
    }
}
