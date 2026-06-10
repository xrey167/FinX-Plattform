//! `OpenBB` Workspace copilot SSE event builders (the response half of the
//! `POST /v1/query` contract).
//!
//! The copilot answers with a `text/event-stream` of typed frames. Each frame
//! is `event: <type>\n` + `data: <json>\n\n`. This module models the documented
//! `openbb-ai` event vocabulary as a single [`SseEvent`] enum whose
//! [`SseEvent::to_sse_frame`] renders the exact wire bytes, so the transport
//! stays a thin writer.
//!
//! # Event vocabulary (public `openbb-ai` SDK docs)
//!
//! Doc source: <https://docs.openbb.co/workspace/copilots> and the `openbb-ai`
//! SDK reference. The frame `event:` names and `data` shapes implemented here:
//!
//! | `event:` name      | `data` payload                                            |
//! |--------------------|-----------------------------------------------------------|
//! | `message_chunk`    | `{ "delta": "<text>" }` — one incremental answer fragment  |
//! | `reasoning_step`   | `{ "event_type":"INFO\|SUCCESS\|WARNING\|ERROR", "message", "details"? }` |
//! | `table`            | `{ "name", "description"?, "data":[{row}] }`               |
//! | `chart`            | `{ "name"?, "type", "data":[…], "x_key", "y_keys":[…] }`   |
//! | `citations`        | `{ "citations":[{ "source_widget_id", "input_arguments"? }] }` |
//! | `get_widget_data`  | `{ "data_sources":[{ "widget_id"/"uuid", "input_arguments"? }] }` |
//!
//! # Documented ambiguities (defensible readings)
//!
//! - **`message_chunk` payload key.** The SDK has used both a bare-string
//!   `data` and a `{ "delta": … }` object across versions; this bridge emits
//!   the object form (`{ "delta": "<text>" }`) because it is unambiguous to
//!   parse and matches the most recent SDK helper. Documented here so a future
//!   reader knows it was a choice, not an accident.
//! - **`get_widget_data` container key.** The published examples name the array
//!   `data_sources`; each entry carries the widget id plus the input arguments
//!   to fetch with. We emit `widget_id` for the id (mirroring the request's
//!   `widget_id`) and `input_arguments` for the params, the names the citations
//!   event also uses, for cross-event consistency.
//! - **`reasoning_step` status field.** Named `event_type` in the SDK
//!   (`INFO`/`SUCCESS`/`WARNING`/`ERROR`); we keep that name and uppercase the
//!   four documented levels.

use serde::Serialize;
use serde_json::{Map, Value};

/// A status level for a [`SseEvent::ReasoningStep`].
///
/// Mirrors the documented `openbb-ai` `event_type` values; serializes to the
/// uppercase token the SDK uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ReasoningStatus {
    /// An informational progress update.
    #[serde(rename = "INFO")]
    Info,
    /// A step completed successfully.
    #[serde(rename = "SUCCESS")]
    Success,
    /// A non-fatal warning.
    #[serde(rename = "WARNING")]
    Warning,
    /// An error the user should see.
    #[serde(rename = "ERROR")]
    Error,
}

impl ReasoningStatus {
    /// The wire token for this status (`INFO` / `SUCCESS` / `WARNING` /
    /// `ERROR`).
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Success => "SUCCESS",
            Self::Warning => "WARNING",
            Self::Error => "ERROR",
        }
    }
}

/// The documented chart kinds a [`SseEvent::Chart`] can render.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartType {
    /// A line chart.
    Line,
    /// A bar chart.
    Bar,
    /// A scatter plot.
    Scatter,
    /// A pie chart.
    Pie,
    /// A donut chart.
    Donut,
}

impl ChartType {
    /// The lowercase wire token for this chart type.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Bar => "bar",
            Self::Scatter => "scatter",
            Self::Pie => "pie",
            Self::Donut => "donut",
        }
    }
}

/// One source attribution in a [`SseEvent::Citations`] event: which widget (and
/// with which input arguments) backed part of the answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Citation {
    /// The id of the widget this citation attributes to.
    pub source_widget_id: String,
    /// The input arguments used when reading the widget, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_arguments: Option<Value>,
}

impl Citation {
    /// Build a citation for `widget_id` with no recorded input arguments.
    #[must_use]
    pub fn new(widget_id: impl Into<String>) -> Self {
        Self {
            source_widget_id: widget_id.into(),
            input_arguments: None,
        }
    }

    /// Build a citation for `widget_id` carrying the `input_arguments` used.
    #[must_use]
    pub fn with_arguments(widget_id: impl Into<String>, input_arguments: Value) -> Self {
        Self {
            source_widget_id: widget_id.into(),
            input_arguments: Some(input_arguments),
        }
    }
}

/// One data source the agent asks the frontend to fetch in a
/// [`SseEvent::GetWidgetData`] event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WidgetDataRequest {
    /// The id of the widget whose data the frontend should fetch and fold back
    /// in as a `tool` message.
    pub widget_id: String,
    /// The input arguments to fetch the widget with, when the agent has them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_arguments: Option<Value>,
}

impl WidgetDataRequest {
    /// Request the frontend fetch `widget_id` with no explicit arguments
    /// (the frontend uses the widget's current params).
    #[must_use]
    pub fn new(widget_id: impl Into<String>) -> Self {
        Self {
            widget_id: widget_id.into(),
            input_arguments: None,
        }
    }

    /// Request the frontend fetch `widget_id` with explicit `input_arguments`.
    #[must_use]
    pub fn with_arguments(widget_id: impl Into<String>, input_arguments: Value) -> Self {
        Self {
            widget_id: widget_id.into(),
            input_arguments: Some(input_arguments),
        }
    }
}

/// A single typed SSE frame in a copilot response stream.
///
/// Render the wire bytes with [`SseEvent::to_sse_frame`]; the `event:` name and
/// `data:` JSON match the documented `openbb-ai` vocabulary (see the module
/// docs for the table and the ambiguity notes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SseEvent {
    /// An incremental fragment of the text answer.
    MessageChunk {
        /// The text fragment to append to the rendered answer.
        delta: String,
    },
    /// A status update describing what the agent is doing.
    ReasoningStep {
        /// The status level.
        status: ReasoningStatus,
        /// The human-readable progress message.
        message: String,
        /// Optional structured details.
        details: Option<Value>,
    },
    /// A table artifact (named rows of homogeneous records).
    Table {
        /// The table's display name.
        name: String,
        /// An optional longer description.
        description: Option<String>,
        /// The row records.
        data: Vec<Value>,
    },
    /// A chart artifact.
    Chart {
        /// Optional chart name.
        name: Option<String>,
        /// The chart kind.
        chart_type: ChartType,
        /// The data points.
        data: Vec<Value>,
        /// The row key mapped to the x axis.
        x_key: String,
        /// The row keys mapped to the y series.
        y_keys: Vec<String>,
    },
    /// A list of source attributions for the answer.
    Citations {
        /// The citations.
        citations: Vec<Citation>,
    },
    /// A request for the frontend to fetch one or more widgets' data, after
    /// which the agent closes the stream (the stateless two-request pattern).
    GetWidgetData {
        /// The widgets whose data the frontend should fetch.
        data_sources: Vec<WidgetDataRequest>,
    },
}

impl SseEvent {
    /// The `event:` name this frame serializes with.
    #[must_use]
    pub const fn event_name(&self) -> &'static str {
        match self {
            Self::MessageChunk { .. } => "message_chunk",
            Self::ReasoningStep { .. } => "reasoning_step",
            Self::Table { .. } => "table",
            Self::Chart { .. } => "chart",
            Self::Citations { .. } => "citations",
            Self::GetWidgetData { .. } => "get_widget_data",
        }
    }

    /// The `data:` JSON payload for this frame.
    #[must_use]
    pub fn data_json(&self) -> Value {
        match self {
            Self::MessageChunk { delta } => {
                let mut map = Map::new();
                map.insert("delta".to_string(), Value::String(delta.clone()));
                Value::Object(map)
            }
            Self::ReasoningStep {
                status,
                message,
                details,
            } => {
                let mut map = Map::new();
                map.insert(
                    "event_type".to_string(),
                    Value::String(status.as_wire().to_string()),
                );
                map.insert("message".to_string(), Value::String(message.clone()));
                if let Some(details) = details {
                    map.insert("details".to_string(), details.clone());
                }
                Value::Object(map)
            }
            Self::Table {
                name,
                description,
                data,
            } => {
                let mut map = Map::new();
                map.insert("name".to_string(), Value::String(name.clone()));
                if let Some(description) = description {
                    map.insert(
                        "description".to_string(),
                        Value::String(description.clone()),
                    );
                }
                map.insert("data".to_string(), Value::Array(data.clone()));
                Value::Object(map)
            }
            Self::Chart {
                name,
                chart_type,
                data,
                x_key,
                y_keys,
            } => {
                let mut map = Map::new();
                if let Some(name) = name {
                    map.insert("name".to_string(), Value::String(name.clone()));
                }
                map.insert(
                    "type".to_string(),
                    Value::String(chart_type.as_wire().to_string()),
                );
                map.insert("data".to_string(), Value::Array(data.clone()));
                map.insert("x_key".to_string(), Value::String(x_key.clone()));
                map.insert(
                    "y_keys".to_string(),
                    Value::Array(
                        y_keys
                            .iter()
                            .map(|key| Value::String(key.clone()))
                            .collect(),
                    ),
                );
                Value::Object(map)
            }
            Self::Citations { citations } => {
                let mut map = Map::new();
                map.insert(
                    "citations".to_string(),
                    serde_json::to_value(citations).unwrap_or(Value::Null),
                );
                Value::Object(map)
            }
            Self::GetWidgetData { data_sources } => {
                let mut map = Map::new();
                map.insert(
                    "data_sources".to_string(),
                    serde_json::to_value(data_sources).unwrap_or(Value::Null),
                );
                Value::Object(map)
            }
        }
    }

    /// Render the full SSE frame bytes: `event: <name>\n` + `data: <json>\n\n`.
    ///
    /// The `data` JSON is compact (single line) so the frame is exactly two
    /// lines plus the terminating blank line, matching the SSE framing the
    /// daemon's other event stream uses.
    #[must_use]
    pub fn to_sse_frame(&self) -> String {
        let data = self.data_json();
        let encoded = serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());
        format!("event: {}\ndata: {}\n\n", self.event_name(), encoded)
    }

    /// Convenience constructor for a [`SseEvent::MessageChunk`].
    #[must_use]
    pub fn message_chunk(delta: impl Into<String>) -> Self {
        Self::MessageChunk {
            delta: delta.into(),
        }
    }

    /// Convenience constructor for a [`SseEvent::ReasoningStep`] without
    /// details.
    #[must_use]
    pub fn reasoning_step(status: ReasoningStatus, message: impl Into<String>) -> Self {
        Self::ReasoningStep {
            status,
            message: message.into(),
            details: None,
        }
    }

    /// Convenience constructor for a [`SseEvent::Citations`] event.
    #[must_use]
    pub const fn citations(citations: Vec<Citation>) -> Self {
        Self::Citations { citations }
    }

    /// Convenience constructor for a [`SseEvent::GetWidgetData`] event.
    #[must_use]
    pub const fn get_widget_data(data_sources: Vec<WidgetDataRequest>) -> Self {
        Self::GetWidgetData { data_sources }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_chunk_frame_is_golden() {
        let frame = SseEvent::message_chunk("Hello").to_sse_frame();
        assert_eq!(
            frame,
            "event: message_chunk\ndata: {\"delta\":\"Hello\"}\n\n"
        );
    }

    #[test]
    fn reasoning_step_frame_is_golden() {
        let frame =
            SseEvent::reasoning_step(ReasoningStatus::Info, "Reading widgets").to_sse_frame();
        assert_eq!(
            frame,
            "event: reasoning_step\ndata: {\"event_type\":\"INFO\",\"message\":\"Reading widgets\"}\n\n"
        );
    }

    #[test]
    fn get_widget_data_frame_is_golden() {
        let event = SseEvent::get_widget_data(vec![WidgetDataRequest::with_arguments(
            "w-1",
            json!({"symbol": "AAPL"}),
        )]);
        let frame = event.to_sse_frame();
        assert_eq!(
            frame,
            "event: get_widget_data\ndata: {\"data_sources\":[{\"input_arguments\":{\"symbol\":\"AAPL\"},\"widget_id\":\"w-1\"}]}\n\n"
        );
    }

    #[test]
    fn citations_frame_is_golden() {
        let event = SseEvent::citations(vec![Citation::with_arguments(
            "w-1",
            json!({"symbol": "AAPL"}),
        )]);
        let frame = event.to_sse_frame();
        assert_eq!(
            frame,
            "event: citations\ndata: {\"citations\":[{\"input_arguments\":{\"symbol\":\"AAPL\"},\"source_widget_id\":\"w-1\"}]}\n\n"
        );
    }

    #[test]
    fn citation_without_arguments_omits_the_key() {
        let event = SseEvent::citations(vec![Citation::new("w-9")]);
        let frame = event.to_sse_frame();
        assert_eq!(
            frame,
            "event: citations\ndata: {\"citations\":[{\"source_widget_id\":\"w-9\"}]}\n\n"
        );
    }

    #[test]
    fn chart_frame_carries_type_and_keys() {
        let event = SseEvent::Chart {
            name: Some("Close".to_string()),
            chart_type: ChartType::Line,
            data: vec![json!({"date": "2024-01-01", "close": 1.0})],
            x_key: "date".to_string(),
            y_keys: vec!["close".to_string()],
        };
        let frame = event.to_sse_frame();
        assert!(frame.starts_with("event: chart\n"));
        assert!(frame.contains("\"type\":\"line\""));
        assert!(frame.contains("\"x_key\":\"date\""));
        assert!(frame.contains("\"y_keys\":[\"close\"]"));
    }

    #[test]
    fn table_frame_includes_rows() {
        let event = SseEvent::Table {
            name: "Prices".to_string(),
            description: None,
            data: vec![json!({"close": 1})],
        };
        let frame = event.to_sse_frame();
        assert!(frame.starts_with("event: table\n"));
        assert!(frame.contains("\"name\":\"Prices\""));
        assert!(frame.contains("\"data\":[{\"close\":1}]"));
        // description omitted when None.
        assert!(!frame.contains("description"));
    }

    #[test]
    fn frame_always_ends_with_blank_line() {
        for event in [
            SseEvent::message_chunk("x"),
            SseEvent::reasoning_step(ReasoningStatus::Success, "ok"),
        ] {
            assert!(event.to_sse_frame().ends_with("\n\n"));
        }
    }
}
