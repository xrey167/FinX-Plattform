//! Integration: a real `tdw-charting` candlestick figure round-trips through the
//! `tdw-chart-host` HTML assembler (G005 — host the spec the charting crate
//! emits).

use serde_json::Value;
use tdw_domain::{MarketDataBar, TimeGranularity};

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

#[test]
fn candlestick_figure_round_trips_through_the_host_page() {
    let bars = vec![
        bar("2026-01-01", 10.0, 10.5, 9.8, 10.0, 1000.0),
        bar("2026-01-02", 10.0, 11.2, 9.9, 11.0, 1100.0),
        bar("2026-01-03", 11.0, 12.4, 10.8, 12.0, 1200.0),
    ];
    // The charting crate emits the Plotly figure spec...
    let figure = tdw_charting::candlestick(&bars);
    assert!(figure.get("data").is_some(), "spec has a data array");

    // ...and the host wraps it in a self-contained HTML page that loads
    // plotly.js and calls Plotly.newPlot with the embedded figure.
    let html = tdw_chart_host::render_html(&figure);
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains(tdw_chart_host::PLOTLY_SCRIPT_SRC));
    assert!(html.contains("Plotly.newPlot"));

    // The embedded figure carries the charting crate's content through verbatim.
    assert!(html.contains("candlestick"), "candlestick trace embedded");
    assert!(html.contains("Volume"), "volume subplot embedded");
    assert!(html.contains("2026-01-03"), "all bars embedded");

    // And it embeds the exact serialized figure (defused for the script tag).
    let figure_json = serde_json::to_string(&figure)
        .expect("serialize figure")
        .replace("</", "<\\/");
    assert!(
        html.contains(&figure_json),
        "host page embeds the exact figure JSON"
    );

    // The round-tripped figure equals what charting produced.
    let reparsed: Value =
        serde_json::from_str(&figure_json.replace("<\\/", "</")).expect("reparse figure");
    assert_eq!(reparsed, figure);
}
