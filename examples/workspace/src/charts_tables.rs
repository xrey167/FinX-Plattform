//! Example 70 — table + chart SSE artifacts from widget/fetch data.
//!
//! A copilot can return more than text: it can stream structured artifacts the
//! Workspace renders inline — a `table` of rows or a `chart`. This example takes
//! a small set of OHLCV bars (the kind a widget-data fetch returns) and emits
//! both: a [`SseEvent::Table`] of the raw rows and a [`SseEvent::Chart`] line
//! over the closes. It also shows how the same bars feed
//! [`tdw_charting::candlestick`] to produce a full Plotly figure-spec, the
//! richer rendering path the REST/SDK surface exposes under `extra.chart`.
//!
//! [`artifacts`] returns the ordered [`SseEvent`]s; [`sse_transcript`] renders
//! them to wire frames; [`plotly_candlestick`] returns the reusable figure-spec.

use serde_json::{Value, json};
use tdw_charting::candlestick;
use tdw_domain::{MarketDataBar, TimeGranularity};
use tdw_openbb_agent::{ChartType, SseEvent};

/// The bars the example renders — three days of AAPL OHLCV, offline and static.
#[must_use]
pub fn bars() -> Vec<MarketDataBar> {
    [
        ("2024-01-02", 183.0, 185.0, 182.5, 184.25, 50_000_000.0),
        ("2024-01-03", 184.5, 186.2, 184.0, 185.60, 48_500_000.0),
        ("2024-01-04", 185.4, 185.9, 183.1, 183.95, 61_200_000.0),
    ]
    .into_iter()
    .map(|(ts, open, high, low, close, volume)| MarketDataBar {
        symbol: "AAPL".to_string(),
        venue: "XNAS".to_string(),
        granularity: TimeGranularity::Day,
        ts: ts.to_string(),
        open,
        high,
        low,
        close,
        volume,
        source: "fileset".to_string(),
    })
    .collect()
}

/// The bars as plain JSON rows — the `data` payload both artifacts carry.
#[must_use]
pub fn rows() -> Vec<Value> {
    bars()
        .iter()
        .map(|bar| {
            json!({
                "date": bar.ts,
                "open": bar.open,
                "high": bar.high,
                "low": bar.low,
                "close": bar.close,
                "volume": bar.volume,
            })
        })
        .collect()
}

/// Build the ordered table + chart artifacts for the bars.
///
/// Emits a `table` of the OHLCV rows, then a `chart` (a line over the closes,
/// keyed on `date`). Both are openbb-ai SSE artifacts the Workspace renders.
#[must_use]
pub fn artifacts() -> Vec<SseEvent> {
    let rows = rows();
    vec![
        SseEvent::Table {
            name: "AAPL OHLCV".to_string(),
            description: Some("Daily bars for AAPL.".to_string()),
            data: rows.clone(),
        },
        SseEvent::Chart {
            name: Some("AAPL Close".to_string()),
            chart_type: ChartType::Line,
            data: rows,
            x_key: "date".to_string(),
            y_keys: vec!["close".to_string()],
        },
    ]
}

/// Render the artifacts to the exact SSE wire frames a transport would write.
#[must_use]
pub fn sse_transcript() -> String {
    artifacts().iter().map(SseEvent::to_sse_frame).collect()
}

/// Build the richer Plotly candlestick figure-spec from the same bars, reusing
/// the shared charting builder (the `extra.chart` payload on the REST surface).
#[must_use]
pub fn plotly_candlestick() -> Value {
    candlestick(&bars())
}
