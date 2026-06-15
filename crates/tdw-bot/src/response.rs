//! The platform-agnostic reply model and its plain-text rendering.
//!
//! A [`BotResponse`] is what the core router produces for every command; the
//! transport adapters (Discord, Telegram) format it for their platform. Keeping
//! the model abstract is what lets the core be unit-tested offline without any
//! bot framework.

use serde::{Deserialize, Serialize};

/// A platform-agnostic reply: the router's output for one command.
///
/// The transport adapters render this for their platform (a Discord message, a
/// Telegram message). Plain text is always available via [`BotResponse::to_text`]
/// so an adapter can fall back to a text reply for any variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BotResponse {
    /// A plain-text reply (a quote summary, a help message, an error).
    Text(String),
    /// A titled key/value table (e.g. a quote's fields, a news list).
    Table(Table),
    /// A chart reply: a one-line caption plus the renderable chart payload.
    Chart(ChartReply),
}

impl BotResponse {
    /// A plain-text reply.
    pub fn text(body: impl Into<String>) -> Self {
        Self::Text(body.into())
    }

    /// Render this response to plain text — the universal fallback every
    /// transport can send. A [`BotResponse::Chart`] renders its caption plus a
    /// note that a chart payload is attached (the binary/spec is sent separately
    /// by the adapter).
    #[must_use]
    pub fn to_text(&self) -> String {
        match self {
            Self::Text(body) => body.clone(),
            Self::Table(table) => table.to_text(),
            Self::Chart(chart) => chart.to_text(),
        }
    }
}

/// A titled table of key/value rows, rendered as aligned `key: value` lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    /// Table title (e.g. `"AAPL quote"`).
    pub title: String,
    /// Ordered `(label, value)` rows.
    pub rows: Vec<(String, String)>,
}

impl Table {
    /// Build a table from a title and its rows.
    pub fn new(title: impl Into<String>, rows: Vec<(String, String)>) -> Self {
        Self {
            title: title.into(),
            rows,
        }
    }

    /// Render the table as a title line followed by left-aligned `label: value`
    /// rows (labels padded to the widest label for a tidy column).
    #[must_use]
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let width = self.rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        let mut out = self.title.clone();
        for (label, value) in &self.rows {
            // Infallible: writing to a String never errors.
            let _ = write!(out, "\n{label:<width$}  {value}");
        }
        out
    }
}

/// The renderable payload a `/chart` command produces.
///
/// `figure_json` is always present — the `tdw-charting` Plotly figure spec, which
/// a chart-host can render. When the crate is built with the `charts` feature,
/// `png` carries a pure-Rust rasterized line chart of the same series; without
/// it, the adapter posts the caption plus the figure-JSON attachment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartReply {
    /// One-line human caption, e.g. `"AAPL — 30 points"`.
    pub caption: String,
    /// The `tdw-charting` Plotly figure JSON (the chart spec).
    pub figure_json: serde_json::Value,
    /// A rasterized PNG of the series, present only when built with `charts`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub png: Option<Vec<u8>>,
}

impl ChartReply {
    /// Render the chart reply to plain text: the caption plus a note describing
    /// what payload is attached (PNG when present, otherwise the figure spec).
    #[must_use]
    pub fn to_text(&self) -> String {
        let attachment = if self.png.is_some() {
            "[chart PNG attached]"
        } else {
            "[chart figure spec attached]"
        };
        format!("{}\n{attachment}", self.caption)
    }
}
