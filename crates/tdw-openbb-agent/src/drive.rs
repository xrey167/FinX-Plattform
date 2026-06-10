//! The pure event sequencer: turn a [`QueryRequest`] plus a streaming model into
//! the ordered [`SseEvent`]s the transport writes.
//!
//! [`answer`] is the single decision point for a copilot turn. It is pure (no
//! I/O of its own — the model's `complete_streaming` is the only side effect)
//! and deterministic given a deterministic model, so the whole two-request flow
//! is golden-testable offline with the
//! [`StubLanguageModel`](tdw_eval_runner::StubLanguageModel)-style stub.
//!
//! # Flow
//!
//! 1. Emit an opening `reasoning_step` (INFO) so the user sees activity.
//! 2. If [`needs_widget_data`] returns a request (first leg of the two-request
//!    pattern — primary widgets present, no folded tool result yet), emit a
//!    `reasoning_step` (INFO) then a `get_widget_data` event and **stop**
//!    ([`Answer::closed_for_widget_data`] is `true`). The transport closes the
//!    stream; the frontend re-POSTs with the folded `tool` result.
//! 3. Otherwise assemble the prompt, drive `complete_streaming`, and emit one
//!    `message_chunk` per fragment. If the conversation carried widget data,
//!    emit a closing `citations` event attributing the answer to the primary
//!    widget(s), preceded by a `SUCCESS` reasoning step.

use tdw_llm::StreamingLanguageModel;

use crate::decision::needs_widget_data;
use crate::event::{Citation, ReasoningStatus, SseEvent};
use crate::prompt::{DEFAULT_MAX_OUTPUT_TOKENS, assemble_chat_request};
use crate::request::QueryRequest;

/// The ordered events for one copilot turn, plus whether the turn closed early
/// to await widget data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// The SSE events to write, in order.
    pub events: Vec<SseEvent>,
    /// `true` when the turn emitted a `get_widget_data` event and stopped (the
    /// first leg of the two-request pattern); the transport closes the stream
    /// and the frontend re-POSTs with the folded result.
    pub closed_for_widget_data: bool,
}

/// Sequence one copilot turn for `request`, driving `model` for the answer text.
///
/// Returns the ordered [`SseEvent`]s and whether the turn closed early to await
/// widget data (see the module docs for the flow). `max_output_tokens` bounds
/// the model's answer; pass [`DEFAULT_MAX_OUTPUT_TOKENS`] for the default.
///
/// On a model error the answer still terminates cleanly: an `ERROR`
/// `reasoning_step` is emitted in place of the message chunks so the user sees a
/// failure rather than a dangling stream.
#[must_use]
pub fn answer_with_budget(
    request: &QueryRequest,
    model: &dyn StreamingLanguageModel,
    max_output_tokens: u32,
) -> Answer {
    let mut events = vec![SseEvent::reasoning_step(
        ReasoningStatus::Info,
        "Reviewing your question and dashboard widgets",
    )];

    if let Some(data_request) = needs_widget_data(request) {
        events.push(SseEvent::reasoning_step(
            ReasoningStatus::Info,
            format!("Fetching data for widget {}", data_request.widget_id),
        ));
        events.push(SseEvent::get_widget_data(vec![data_request]));
        return Answer {
            events,
            closed_for_widget_data: true,
        };
    }

    let chat = assemble_chat_request(request, max_output_tokens);
    let mut chunks: Vec<String> = Vec::new();
    let result = model.complete_streaming(&chat, &mut |chunk| {
        chunks.push(chunk.to_string());
    });

    match result {
        Ok(_) => {
            for chunk in chunks {
                events.push(SseEvent::message_chunk(chunk));
            }
            if request.has_tool_result() {
                events.push(SseEvent::reasoning_step(
                    ReasoningStatus::Success,
                    "Answered using fetched widget data",
                ));
                let citations = citations_for(request);
                if !citations.is_empty() {
                    events.push(SseEvent::citations(citations));
                }
            }
        }
        Err(error) => {
            events.push(SseEvent::reasoning_step(
                ReasoningStatus::Error,
                format!("Model error: {error}"),
            ));
        }
    }

    Answer {
        events,
        closed_for_widget_data: false,
    }
}

/// Sequence one copilot turn with the [`DEFAULT_MAX_OUTPUT_TOKENS`] budget.
#[must_use]
pub fn answer(request: &QueryRequest, model: &dyn StreamingLanguageModel) -> Answer {
    answer_with_budget(request, model, DEFAULT_MAX_OUTPUT_TOKENS)
}

/// Build citations attributing the answer to the primary widgets (each carrying
/// its current params as the input arguments).
fn citations_for(request: &QueryRequest) -> Vec<Citation> {
    request
        .widgets
        .primary
        .iter()
        .filter_map(|widget| {
            let id = widget.id()?.to_string();
            let citation = match &widget.params {
                serde_json::Value::Object(map) if !map.is_empty() => {
                    Citation::with_arguments(id, serde_json::Value::Object(map.clone()))
                }
                _ => Citation::new(id),
            };
            Some(citation)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{Message, MessageRole, Widget, Widgets};
    use serde_json::json;
    use tdw_llm::{
        ChatMessage, ChatRequest, ChatResponse, LanguageModel, MessageRole as LlmRole, Usage,
        split_into_chunks,
    };

    /// A deterministic offline stub mirroring `tdw-eval-runner::StubLanguageModel`
    /// (kept local so the pure crate needs no test-only dep): echoes the joined
    /// message contents, streamed in three chunks.
    #[derive(Default)]
    struct EchoStub;

    impl LanguageModel for EchoStub {
        fn model_id(&self) -> &'static str {
            "echo-stub"
        }

        fn complete(&self, request: ChatRequest) -> tdw_llm::Result<ChatResponse> {
            let _ = tdw_llm::last_user_message(&request)?;
            let content = request
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            Ok(ChatResponse {
                model_id: self.model_id().to_string(),
                message: ChatMessage {
                    role: LlmRole::Assistant,
                    content,
                },
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
            })
        }
    }

    impl StreamingLanguageModel for EchoStub {
        fn complete_streaming(
            &self,
            request: &ChatRequest,
            on_chunk: &mut dyn FnMut(&str),
        ) -> tdw_llm::Result<ChatResponse> {
            let response = self.complete(request.clone())?;
            for chunk in split_into_chunks(&response.message.content, 3) {
                on_chunk(&chunk);
            }
            Ok(response)
        }
    }

    fn human(text: &str) -> Message {
        Message {
            role: MessageRole::Human,
            content: json!(text),
        }
    }

    #[test]
    fn first_leg_emits_reasoning_then_get_widget_data_and_closes() {
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
        let result = answer(&request, &EchoStub);
        assert!(result.closed_for_widget_data);
        // reasoning_step (INFO), reasoning_step (INFO), get_widget_data.
        assert_eq!(result.events.len(), 3);
        assert_eq!(result.events[0].event_name(), "reasoning_step");
        assert_eq!(result.events[2].event_name(), "get_widget_data");
        // No message chunks were emitted on the first leg.
        assert!(
            result
                .events
                .iter()
                .all(|event| event.event_name() != "message_chunk")
        );
    }

    #[test]
    fn second_leg_streams_chunks_and_emits_citations() {
        let request = QueryRequest {
            messages: vec![
                human("How did AAPL do?"),
                Message {
                    role: MessageRole::Tool,
                    content: json!({"rows": [{"close": 190.0}]}),
                },
            ],
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
        let result = answer(&request, &EchoStub);
        assert!(!result.closed_for_widget_data);
        let names: Vec<&str> = result.events.iter().map(SseEvent::event_name).collect();
        assert_eq!(names.first(), Some(&"reasoning_step"));
        assert!(names.contains(&"message_chunk"), "streams the answer");
        assert_eq!(names.last(), Some(&"citations"), "closes with citations");
        // The citation attributes to the primary widget with its params.
        let citation_event = result.events.last().expect("citations event");
        let data = citation_event.data_json();
        assert_eq!(data["citations"][0]["source_widget_id"], "w-1");
        assert_eq!(data["citations"][0]["input_arguments"]["symbol"], "AAPL");
    }

    #[test]
    fn no_widgets_answers_directly_without_citations() {
        let request = QueryRequest {
            messages: vec![human("What is a P/E ratio?")],
            ..QueryRequest::default()
        };
        let result = answer(&request, &EchoStub);
        assert!(!result.closed_for_widget_data);
        let names: Vec<&str> = result.events.iter().map(SseEvent::event_name).collect();
        assert!(names.contains(&"message_chunk"));
        assert!(
            !names.contains(&"citations"),
            "no widget data, no citations"
        );
        assert!(!names.contains(&"get_widget_data"));
    }

    #[test]
    fn message_chunks_reconstruct_the_full_answer() {
        let request = QueryRequest {
            messages: vec![human("hello world")],
            ..QueryRequest::default()
        };
        let result = answer(&request, &EchoStub);
        let streamed: String = result
            .events
            .iter()
            .filter_map(|event| match event {
                SseEvent::MessageChunk { delta } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        // The stub echoes system + user content; the user's text is present.
        assert!(streamed.contains("hello world"));
    }
}
