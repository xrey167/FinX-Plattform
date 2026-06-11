#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Pure-Rust server-side chart-spec builders (gap-matrix item **L5.5**).
//!
//! When a chartable route is called with `chart=true`, the dispatcher attaches a
//! renderable chart spec to the [`tdw_domain::ResultEnvelope`]'s `chart` slot.
//! This crate produces that spec as a **Plotly figure** — the JSON object
//! `{ "data": [ ...traces ], "layout": { ... } }` that a plotly.js client renders
//! directly in the browser. Nothing here renders pixels: the spec is data, built
//! with plain `serde_json`, so the warehouse pulls in ZERO new native graphics
//! dependency.
//!
//! ## Figure shape (clean-room)
//!
//! The figure object, the trace `type`s (`candlestick`, `scatter` with
//! `mode: "lines"`, `bar`), and the `x` / `open` / `high` / `low` / `close` /
//! `y` data keys are the public plotly.js graphing schema, documented at
//! <https://plotly.com/javascript/candlestick-charts/>,
//! <https://plotly.com/javascript/line-charts/>, and
//! <https://plotly.com/javascript/bar-charts/>. No reference implementation was
//! consulted; only the public figure-shape docs.
//!
//! ## Determinism
//!
//! Every trace and the layout are assembled as a [`serde_json::Map`] with keys
//! inserted in a fixed, sorted order and the data arrays following the input bar
//! order, so two runs over the same input serialize byte-for-byte identically.
//! The golden snapshot tests pin the exact JSON per builder.
//!
//! ## Inputs
//!
//! The OHLCV builders consume already-parsed [`tdw_domain::MarketDataBar`]s — the
//! caller (the dispatcher) reuses the warehouse's single tolerant OHLCV row
//! parser (`tdw_service_api::technical_compute::parse_bars`) rather than this
//! crate re-deriving a second bar shape. The line builder consumes [`LinePoint`]s
//! (a `date` + optional numeric `value`), the shape every single-series row
//! (`MacroSeries` / `RateObservation` / `YieldCurvePoint`) reduces to.

use serde_json::{Map, Value};
use tdw_domain::MarketDataBar;

/// One point of a single-value time series for [`line`].
///
/// `x` is the observation date (or any category label); `y` is the observed
/// value, `None` for a missing observation (rendered as a gap by plotly.js).
#[derive(Clone, Debug, PartialEq)]
pub struct LinePoint {
    /// X-axis category — typically an observation date string.
    pub x: String,
    /// Y-axis value, `None` for a missing observation.
    pub y: Option<f64>,
}

impl LinePoint {
    /// Construct a line point from a date label and an optional value.
    #[must_use]
    pub fn new(x: impl Into<String>, y: Option<f64>) -> Self {
        Self { x: x.into(), y }
    }
}

/// A named overlay series for [`indicator_overlay`]: a line trace drawn on top
/// of the candlestick, positionally date-aligned with the source bars.
///
/// `values` is the indicator output column; it should be the same length as the
/// bar slice it overlays (the `technical/*` compute path returns one row per
/// bar). A `None` value is a gap (e.g. the leading warm-up region of a moving
/// average).
#[derive(Clone, Debug, PartialEq)]
pub struct OverlaySeries {
    /// Legend/trace name, e.g. `"sma"` or `"upper"`.
    pub name: String,
    /// One value per source bar, `None` for a gap.
    pub values: Vec<Option<f64>>,
}

impl OverlaySeries {
    /// Construct an overlay series from a name and its per-bar values.
    #[must_use]
    pub fn new(name: impl Into<String>, values: Vec<Option<f64>>) -> Self {
        Self {
            name: name.into(),
            values,
        }
    }
}

/// Build a Plotly figure for a candlestick chart from OHLCV `bars`.
///
/// Emits a single `candlestick` trace over the bars' timestamps; when any bar
/// carries a non-zero volume, a `bar` volume trace on a secondary y-axis (`y2`)
/// and the matching `yaxis2` layout block are added so volume renders as a
/// subplot below the price panel.
///
/// The returned value is the figure object `{ "data": [...], "layout": {...} }`.
#[must_use]
pub fn candlestick(bars: &[MarketDataBar]) -> Value {
    let x: Vec<Value> = bars.iter().map(|b| Value::from(b.ts.clone())).collect();

    let mut data = vec![candlestick_trace(bars, &x)];

    let has_volume = bars.iter().any(|b| b.volume != 0.0);
    let mut layout = base_layout("Candlestick");
    if has_volume {
        data.push(volume_trace(bars, &x));
        attach_volume_axes(&mut layout);
    }

    figure(data, layout)
}

/// Build a Plotly figure for a single-value line (scatter) chart from `points`.
///
/// Emits one `scatter` trace in `lines` mode over the points' `x` labels and
/// `y` values, with `None` values serialized as JSON `null` so plotly.js draws a
/// gap rather than interpolating across missing observations.
#[must_use]
pub fn line(points: &[LinePoint]) -> Value {
    let x: Vec<Value> = points.iter().map(|p| Value::from(p.x.clone())).collect();
    let y: Vec<Value> = points.iter().map(|p| number_or_null(p.y)).collect();

    let mut trace = Map::new();
    trace.insert("mode".to_string(), Value::from("lines"));
    trace.insert("name".to_string(), Value::from("value"));
    trace.insert("type".to_string(), Value::from("scatter"));
    trace.insert("x".to_string(), Value::Array(x));
    trace.insert("y".to_string(), Value::Array(y));

    figure(vec![Value::Object(trace)], base_layout("Line"))
}

/// Build a Plotly figure that overlays indicator line trace(s) on top of a
/// candlestick of the source `bars`.
///
/// The candlestick (and its volume subplot, when present) is built exactly as
/// [`candlestick`] does; each [`OverlaySeries`] is appended as a `scatter` line
/// trace positionally aligned to the bars' timestamps. An overlay shorter than
/// the bar slice is padded with trailing `null`s; a longer one is truncated, so
/// the x/y arrays always agree in length.
#[must_use]
pub fn indicator_overlay(bars: &[MarketDataBar], overlays: &[OverlaySeries]) -> Value {
    let x: Vec<Value> = bars.iter().map(|b| Value::from(b.ts.clone())).collect();

    let has_volume = bars.iter().any(|b| b.volume != 0.0);
    let mut data = vec![candlestick_trace(bars, &x)];
    if has_volume {
        data.push(volume_trace(bars, &x));
    }

    for overlay in overlays {
        let y: Vec<Value> = (0..bars.len())
            .map(|i| number_or_null(overlay.values.get(i).copied().flatten()))
            .collect();
        let mut trace = Map::new();
        trace.insert("mode".to_string(), Value::from("lines"));
        trace.insert("name".to_string(), Value::from(overlay.name.clone()));
        trace.insert("type".to_string(), Value::from("scatter"));
        trace.insert("x".to_string(), Value::Array(x.clone()));
        trace.insert("y".to_string(), Value::Array(y));
        data.push(Value::Object(trace));
    }

    let mut layout = base_layout("Indicator overlay");
    if has_volume {
        attach_volume_axes(&mut layout);
    }
    figure(data, layout)
}

// --- Trace / layout primitives ----------------------------------------------

/// Assemble the top-level figure object from its traces and layout.
fn figure(data: Vec<Value>, layout: Value) -> Value {
    let mut figure = Map::new();
    figure.insert("data".to_string(), Value::Array(data));
    figure.insert("layout".to_string(), layout);
    Value::Object(figure)
}

/// Build the OHLC `candlestick` trace over a precomputed `x` axis.
fn candlestick_trace(bars: &[MarketDataBar], x: &[Value]) -> Value {
    let mut trace = Map::new();
    trace.insert("close".to_string(), float_array(bars, |b| b.close));
    trace.insert("high".to_string(), float_array(bars, |b| b.high));
    trace.insert("low".to_string(), float_array(bars, |b| b.low));
    trace.insert("name".to_string(), Value::from("OHLC"));
    trace.insert("open".to_string(), float_array(bars, |b| b.open));
    trace.insert("type".to_string(), Value::from("candlestick"));
    trace.insert("x".to_string(), Value::Array(x.to_vec()));
    Value::Object(trace)
}

/// Build the volume `bar` trace on the secondary y-axis over `x`.
fn volume_trace(bars: &[MarketDataBar], x: &[Value]) -> Value {
    let mut trace = Map::new();
    trace.insert("name".to_string(), Value::from("Volume"));
    trace.insert("type".to_string(), Value::from("bar"));
    trace.insert("x".to_string(), Value::Array(x.to_vec()));
    trace.insert("y".to_string(), float_array(bars, |b| b.volume));
    trace.insert("yaxis".to_string(), Value::from("y2"));
    Value::Object(trace)
}

/// The shared layout block for a price-style figure with a given `title`.
fn base_layout(title: &str) -> Value {
    let mut layout = Map::new();
    layout.insert("dragmode".to_string(), Value::from("zoom"));
    layout.insert("showlegend".to_string(), Value::Bool(true));
    layout.insert("template".to_string(), Value::from("plotly_dark"));
    layout.insert("title".to_string(), Value::from(title));
    layout.insert("xaxis".to_string(), x_axis());
    layout.insert("yaxis".to_string(), y_axis(0.3));
    Value::Object(layout)
}

/// The primary x-axis: a range slider lets the client scrub the time series.
fn x_axis() -> Value {
    let mut axis = Map::new();
    let mut slider = Map::new();
    slider.insert("visible".to_string(), Value::Bool(true));
    axis.insert("rangeslider".to_string(), Value::Object(slider));
    axis.insert("type".to_string(), Value::from("date"));
    Value::Object(axis)
}

/// The price y-axis, anchored above the volume subplot at `domain_floor`.
fn y_axis(domain_floor: f64) -> Value {
    let mut axis = Map::new();
    axis.insert(
        "domain".to_string(),
        Value::Array(vec![Value::from(domain_floor), Value::from(1.0)]),
    );
    axis.insert("title".to_string(), Value::from("Price"));
    Value::Object(axis)
}

/// Add the secondary (`yaxis2`) volume axis and reshape the price axis domain so
/// price and volume share the panel without overlap.
fn attach_volume_axes(layout: &mut Value) {
    let Value::Object(map) = layout else {
        return;
    };
    let mut volume_axis = Map::new();
    volume_axis.insert(
        "domain".to_string(),
        Value::Array(vec![Value::from(0.0), Value::from(0.2)]),
    );
    volume_axis.insert("title".to_string(), Value::from("Volume"));
    map.insert("yaxis2".to_string(), Value::Object(volume_axis));
}

/// Project one `f64` column of `bars` into a JSON array via `pick`.
fn float_array(bars: &[MarketDataBar], pick: impl Fn(&MarketDataBar) -> f64) -> Value {
    Value::Array(bars.iter().map(|b| number_or_null(Some(pick(b)))).collect())
}

/// Convert an optional finite `f64` to a JSON number, or `null` when absent or
/// non-finite (NaN / ±∞ have no JSON number form).
fn number_or_null(value: Option<f64>) -> Value {
    value
        .filter(|v| v.is_finite())
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_domain::TimeGranularity;

    fn bar(ts: &str, open: f64, high: f64, low: f64, close: f64, volume: f64) -> MarketDataBar {
        MarketDataBar {
            symbol: "AAPL".to_string(),
            venue: "XNAS".to_string(),
            granularity: TimeGranularity::Day,
            ts: ts.to_string(),
            open,
            high,
            low,
            close,
            volume,
            source: "fixture".to_string(),
        }
    }

    fn ohlcv_fixture() -> Vec<MarketDataBar> {
        vec![
            bar("2026-01-01", 10.0, 10.5, 9.8, 10.0, 1000.0),
            bar("2026-01-02", 10.0, 11.2, 9.9, 11.0, 1100.0),
            bar("2026-01-03", 11.0, 12.4, 10.8, 12.0, 1200.0),
        ]
    }

    fn ohlc_only_fixture() -> Vec<MarketDataBar> {
        vec![
            bar("2026-01-01", 10.0, 10.5, 9.8, 10.0, 0.0),
            bar("2026-01-02", 10.0, 11.2, 9.9, 11.0, 0.0),
        ]
    }

    #[test]
    fn candlestick_with_volume_matches_golden() {
        let figure = candlestick(&ohlcv_fixture());
        let json = serde_json::to_string_pretty(&figure).expect("serialize");
        let expected = include_str!("../tests/golden/candlestick_volume.json").trim_end();
        assert_eq!(json, expected);
    }

    #[test]
    fn candlestick_without_volume_omits_volume_trace_and_axis() {
        let figure = candlestick(&ohlc_only_fixture());
        let json = serde_json::to_string_pretty(&figure).expect("serialize");
        let expected = include_str!("../tests/golden/candlestick_no_volume.json").trim_end();
        assert_eq!(json, expected);
    }

    #[test]
    fn line_matches_golden_with_gap() {
        let points = vec![
            LinePoint::new("2026-01-01", Some(3.1)),
            LinePoint::new("2026-01-02", None),
            LinePoint::new("2026-01-03", Some(3.4)),
        ];
        let figure = line(&points);
        let json = serde_json::to_string_pretty(&figure).expect("serialize");
        let expected = include_str!("../tests/golden/line.json").trim_end();
        assert_eq!(json, expected);
    }

    #[test]
    fn indicator_overlay_matches_golden() {
        let overlays = vec![OverlaySeries::new(
            "sma",
            vec![None, Some(10.5), Some(11.5)],
        )];
        let figure = indicator_overlay(&ohlcv_fixture(), &overlays);
        let json = serde_json::to_string_pretty(&figure).expect("serialize");
        let expected = include_str!("../tests/golden/indicator_overlay.json").trim_end();
        assert_eq!(json, expected);
    }

    #[test]
    fn builders_are_deterministic() {
        let bars = ohlcv_fixture();
        assert_eq!(candlestick(&bars), candlestick(&bars));
        let pts = [LinePoint::new("d", Some(1.0))];
        assert_eq!(line(&pts), line(&pts));
    }

    #[test]
    fn overlay_shorter_than_bars_is_padded_with_null() {
        let bars = ohlcv_fixture();
        let overlays = vec![OverlaySeries::new("short", vec![Some(1.0)])];
        let figure = indicator_overlay(&bars, &overlays);
        let traces = figure["data"].as_array().expect("data array");
        let overlay = traces.last().expect("overlay trace");
        let y = overlay["y"].as_array().expect("y array");
        assert_eq!(y.len(), bars.len());
        assert_eq!(y[1], Value::Null);
        assert_eq!(y[2], Value::Null);
    }

    #[test]
    fn non_finite_values_serialize_as_null() {
        let points = vec![
            LinePoint::new("a", Some(f64::NAN)),
            LinePoint::new("b", Some(f64::INFINITY)),
        ];
        let figure = line(&points);
        let y = figure["data"][0]["y"].as_array().expect("y array");
        assert_eq!(y[0], Value::Null);
        assert_eq!(y[1], Value::Null);
    }
}
