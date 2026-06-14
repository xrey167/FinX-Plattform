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
//! | `citations`        | `{ "citations":[{ "source_widget_id", "input_arguments"?, "extra_citations"?, "source_info"? }] }` |
//! | `get_widget_data`  | `{ "data_sources":[{ "widget_id"/"uuid", "input_arguments"? }] }` |
//! | `html`             | `{ "name"?, "html":"<…>" }` — a custom rendered HTML artifact |
//! | `prompt_suggestions`| `{ "suggestions":["<prompt>", …] }` — follow-up prompts at stream end |
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

/// Page-anchored source detail for a [`Citation`] backed by a document (e.g. a
/// PDF the user attached as context, examples 35/36).
///
/// Mirrors the `openbb-ai` PDF-citation `source_info` shape: a free-form
/// document name/uri plus an optional 1-based page number the cited passage was
/// drawn from. Both fields serialize only when present so a widget-only citation
/// stays the lean `{ source_widget_id, … }` shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceInfo {
    /// The document this citation is drawn from (a file name, title, or uri).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The source kind (e.g. `"document"`, `"pdf"`, `"web"`), when known.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    /// The 1-based page number the cited passage was drawn from, for paginated
    /// documents.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

impl SourceInfo {
    /// A document `source_info` of `type: "document"` for `name`, page-anchored
    /// at the 1-based `page`.
    #[must_use]
    pub fn document_page(name: impl Into<String>, page: u32) -> Self {
        Self {
            name: Some(name.into()),
            source_type: Some("document".to_string()),
            page: Some(page),
        }
    }
}

/// One source attribution in a [`SseEvent::Citations`] event: which widget (and
/// with which input arguments) backed part of the answer.
///
/// Beyond the widget binding the citation can carry the richer `openbb-ai`
/// fields (gap #7): `extra_citations` (additional widget ids that also backed
/// the same claim) and `source_info` (page-anchored document detail for a
/// PDF/file-backed citation, examples 35/36). All extra fields serialize only
/// when set, so a plain widget citation keeps its original wire shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Citation {
    /// The id of the widget this citation attributes to.
    pub source_widget_id: String,
    /// The input arguments used when reading the widget, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_arguments: Option<Value>,
    /// Additional widget ids that also backed this same claim (so one citation
    /// can attribute to several sources without emitting a citation each).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extra_citations: Vec<String>,
    /// Page-anchored document detail when this citation is backed by a file/PDF
    /// context item rather than (or in addition to) a dashboard widget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_info: Option<SourceInfo>,
}

impl Citation {
    /// Build a citation for `widget_id` with no recorded input arguments.
    #[must_use]
    pub fn new(widget_id: impl Into<String>) -> Self {
        Self {
            source_widget_id: widget_id.into(),
            input_arguments: None,
            extra_citations: Vec::new(),
            source_info: None,
        }
    }

    /// Build a citation for `widget_id` carrying the `input_arguments` used.
    #[must_use]
    pub fn with_arguments(widget_id: impl Into<String>, input_arguments: Value) -> Self {
        Self {
            source_widget_id: widget_id.into(),
            input_arguments: Some(input_arguments),
            extra_citations: Vec::new(),
            source_info: None,
        }
    }

    /// Attach additional widget ids that also backed this same claim.
    #[must_use]
    pub fn with_extra_citations(mut self, extra: Vec<String>) -> Self {
        self.extra_citations = extra;
        self
    }

    /// Attach page-anchored document `source_info` (a file/PDF-backed citation).
    #[must_use]
    pub fn with_source_info(mut self, source_info: SourceInfo) -> Self {
        self.source_info = Some(source_info);
        self
    }

    /// Build a document-backed citation: `source_widget_id` is the document
    /// name (so widget-keyed renderers still show a source), and the
    /// page-anchored detail is carried in `source_info`.
    #[must_use]
    pub fn document(name: impl Into<String>, page: u32) -> Self {
        let name = name.into();
        Self {
            source_widget_id: name.clone(),
            input_arguments: None,
            extra_citations: Vec::new(),
            source_info: Some(SourceInfo::document_page(name, page)),
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
    /// A custom rendered HTML artifact (the `openbb-ai` HTML-artifact output,
    /// example 39): an arbitrary HTML fragment Workspace renders inline.
    HtmlArtifact {
        /// Optional artifact name shown above the rendered HTML.
        name: Option<String>,
        /// The raw HTML to render.
        html: String,
    },
    /// A list of follow-up prompts to suggest after the answer (the `openbb-ai`
    /// `prompt_suggestions` output, example UX parity): emitted once at stream
    /// end so the UI can offer the user next questions.
    PromptSuggestions {
        /// The suggested follow-up prompts, in display order.
        suggestions: Vec<String>,
    },
}

/// Render the `data:` JSON for a [`SseEvent::Chart`] frame (extracted so
/// [`SseEvent::data_json`] stays within the line budget).
fn chart_json(
    name: Option<&str>,
    chart_type: ChartType,
    data: &[Value],
    x_key: &str,
    y_keys: &[String],
) -> Value {
    let mut map = Map::new();
    if let Some(name) = name {
        map.insert("name".to_string(), Value::String(name.to_string()));
    }
    map.insert(
        "type".to_string(),
        Value::String(chart_type.as_wire().to_string()),
    );
    map.insert("data".to_string(), Value::Array(data.to_vec()));
    map.insert("x_key".to_string(), Value::String(x_key.to_string()));
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
            Self::HtmlArtifact { .. } => "html",
            Self::PromptSuggestions { .. } => "prompt_suggestions",
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
            } => chart_json(name.as_deref(), *chart_type, data, x_key, y_keys),
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
            Self::HtmlArtifact { name, html } => {
                let mut map = Map::new();
                if let Some(name) = name {
                    map.insert("name".to_string(), Value::String(name.clone()));
                }
                map.insert("html".to_string(), Value::String(html.clone()));
                Value::Object(map)
            }
            Self::PromptSuggestions { suggestions } => {
                let mut map = Map::new();
                map.insert(
                    "suggestions".to_string(),
                    Value::Array(
                        suggestions
                            .iter()
                            .map(|prompt| Value::String(prompt.clone()))
                            .collect(),
                    ),
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

    /// Convenience constructor for a named [`SseEvent::HtmlArtifact`].
    #[must_use]
    pub fn html_artifact(name: impl Into<String>, html: impl Into<String>) -> Self {
        Self::HtmlArtifact {
            name: Some(name.into()),
            html: html.into(),
        }
    }

    /// Convenience constructor for a [`SseEvent::PromptSuggestions`] event from
    /// any iterable of prompt strings.
    #[must_use]
    pub fn prompt_suggestions<I, S>(suggestions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::PromptSuggestions {
            suggestions: suggestions.into_iter().map(Into::into).collect(),
        }
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
    fn prompt_suggestions_frame_is_golden() {
        let event = SseEvent::prompt_suggestions(["What about MSFT?", "Show me the chart"]);
        let frame = event.to_sse_frame();
        assert_eq!(
            frame,
            "event: prompt_suggestions\ndata: {\"suggestions\":[\"What about MSFT?\",\"Show me the chart\"]}\n\n"
        );
    }

    #[test]
    fn html_artifact_frame_carries_name_and_html() {
        let event = SseEvent::html_artifact("Summary", "<b>hi</b>");
        let frame = event.to_sse_frame();
        // serde_json serializes object keys in sorted order (html before name).
        assert_eq!(
            frame,
            "event: html\ndata: {\"html\":\"<b>hi</b>\",\"name\":\"Summary\"}\n\n"
        );
    }

    #[test]
    fn html_artifact_omits_name_when_absent() {
        let event = SseEvent::HtmlArtifact {
            name: None,
            html: "<p>x</p>".to_string(),
        };
        let frame = event.to_sse_frame();
        assert_eq!(frame, "event: html\ndata: {\"html\":\"<p>x</p>\"}\n\n");
    }

    #[test]
    fn citation_with_extra_and_source_info_serializes_the_rich_fields() {
        let citation = Citation::with_arguments("w-1", json!({"symbol": "AAPL"}))
            .with_extra_citations(vec!["w-2".to_string()])
            .with_source_info(SourceInfo::document_page("10-K.pdf", 42));
        let event = SseEvent::citations(vec![citation]);
        let data = event.data_json();
        let entry = &data["citations"][0];
        assert_eq!(entry["source_widget_id"], "w-1");
        assert_eq!(entry["extra_citations"][0], "w-2");
        assert_eq!(entry["source_info"]["name"], "10-K.pdf");
        assert_eq!(entry["source_info"]["type"], "document");
        assert_eq!(entry["source_info"]["page"], 42);
    }

    #[test]
    fn plain_citation_omits_the_rich_fields() {
        let event = SseEvent::citations(vec![Citation::new("w-9")]);
        let frame = event.to_sse_frame();
        // No empty extra_citations array, no null source_info.
        assert!(!frame.contains("extra_citations"));
        assert!(!frame.contains("source_info"));
    }

    #[test]
    fn document_citation_is_page_anchored() {
        let event = SseEvent::citations(vec![Citation::document("filing.pdf", 7)]);
        let data = event.data_json();
        let entry = &data["citations"][0];
        assert_eq!(entry["source_widget_id"], "filing.pdf");
        assert_eq!(entry["source_info"]["page"], 7);
    }

    #[test]
    fn frame_always_ends_with_blank_line() {
        for event in [
            SseEvent::message_chunk("x"),
            SseEvent::reasoning_step(ReasoningStatus::Success, "ok"),
            SseEvent::html_artifact("a", "<i>b</i>"),
            SseEvent::prompt_suggestions(["next?"]),
        ] {
            assert!(event.to_sse_frame().ends_with("\n\n"));
        }
    }
}
