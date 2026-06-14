#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
//! Native desktop render HOST for `tdw-charting` Plotly figures
//! (openbb-ecosystem-p1 item **G005** — the `PyWry`-equivalent).
//!
//! [`tdw-charting`](https://docs.rs/tdw-charting) emits the chart *spec*: a
//! Plotly **figure** JSON object `{ "data": [...traces], "layout": {...} }`. This
//! crate is the render *host* that turns that spec into something a human can
//! look at, in two layers:
//!
//! 1. **The HTML spec assembler (default, pure Rust).** [`render_html`] wraps a
//!    figure JSON in a self-contained HTML page — a templated plotly.js host page
//!    that embeds the figure verbatim and calls `Plotly.newPlot` to draw it.
//!    This layer has ZERO native dependency, is fully unit-testable, and is the
//!    only thing the default workspace build compiles.
//! 2. **The native window (the `gui` feature, optional).** With `--features gui`,
//!    [`show`] / [`open_window`] open that HTML in a real desktop window via a
//!    [`tao`] event loop + a [`wry`] `WebViewBuilder` — exactly what `PyWry` does
//!    (`PyWry` literally wraps Rust `wry`).
//!
//! # CI-safety boundary
//!
//! `wry` + `tao` pull the host platform's native webview (`WebView2` on Windows,
//! `WebKitGTK` on Linux, `WKWebView` on macOS), which a CI runner may not have.
//! They are therefore **optional dependencies behind the non-default `gui`
//! feature**, and every line that touches them is guarded by
//! `#[cfg(feature = "gui")]`. A plain `cargo build/clippy/test --workspace`
//! exercises only the spec assembler below and never compiles wry/tao — so the
//! core CI stays green without any system webview library. The desktop window
//! requires `--features gui` **and** a platform webview at build+run time.
//!
//! # Plotly.js host page (clean-room)
//!
//! The host page is the documented plotly.js getting-started shape: a `<div>`
//! render target plus `Plotly.newPlot(target, figure.data, figure.layout)`. See
//! <https://plotly.com/javascript/getting-started/>. No reference implementation
//! was consulted; only the public plotly.js embedding docs.

use serde_json::Value;

/// The plotly.js bundle URL embedded as a `<script src=…>` in the host page.
///
/// A CDN reference (not a vendored multi-megabyte bundle) keeps this crate a
/// small pure-Rust spec assembler; a deployment that must render fully offline
/// can post-process the page to inline its own plotly.js copy.
pub const PLOTLY_SCRIPT_SRC: &str = "https://cdn.plot.ly/plotly-2.35.2.min.js";

/// The DOM id of the `<div>` plotly.js renders the figure into.
const RENDER_TARGET_ID: &str = "tdw-chart";

/// Assemble a self-contained HTML page that renders `figure` with plotly.js.
///
/// `figure` is a [`tdw-charting`](https://docs.rs/tdw-charting) Plotly figure —
/// the object `{ "data": [...], "layout": {...} }`. The returned string is a
/// complete HTML document: a `<head>` that loads plotly.js via a `<script>` tag
/// and a `<body>` with a single render-target `<div>` followed by an inline
/// `<script>` that embeds the figure JSON and calls
/// `Plotly.newPlot(target, figure.data, figure.layout)`.
///
/// The embedded figure JSON is produced with [`serde_json::to_string`], so it is
/// valid JSON and therefore a valid JavaScript object literal; a `</script>`
/// sequence cannot appear inside serialized JSON (the `/` is not escaped but the
/// pair only arises in string content, which JSON escaping leaves intact), and
/// to be safe any literal `/` that would close the script tag is neutralized.
#[must_use]
pub fn render_html(figure: &Value) -> String {
    let title = figure_title(figure).unwrap_or("Chart");
    // Serialize the figure and defuse a `</script>` break-out: split the `<`
    // from a following `/` so the browser never sees a closing script tag inside
    // the embedded JSON, while the JSON value parsed by `JSON.parse` is
    // unchanged (the `\/` escape is a valid JSON string escape for `/`).
    let figure_json = serde_json::to_string(figure)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</", "<\\/");

    format!(
        "<!DOCTYPE html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title}</title>\n\
         <script src=\"{script}\" charset=\"utf-8\"></script>\n\
         <style>html,body{{margin:0;height:100%;background:#111}}#{target}{{width:100%;height:100%}}</style>\n\
         </head>\n\
         <body>\n\
         <div id=\"{target}\"></div>\n\
         <script>\n\
         const figure = JSON.parse({figure_literal});\n\
         Plotly.newPlot(\"{target}\", figure.data, figure.layout, {{responsive:true}});\n\
         </script>\n\
         </body>\n\
         </html>\n",
        title = html_escape(title),
        script = PLOTLY_SCRIPT_SRC,
        target = RENDER_TARGET_ID,
        // Embed the figure JSON as a JS *string* literal and let `JSON.parse`
        // rebuild the object — avoids any ambiguity around embedding a raw
        // object literal and keeps the figure bytes intact.
        figure_literal = js_string_literal(&figure_json),
    )
}

/// Extract the figure's `layout.title` text for the page `<title>`, when present
/// as a plain string (plotly titles can also be an object — only the simple
/// string form is used here).
fn figure_title(figure: &Value) -> Option<&str> {
    figure.get("layout")?.get("title")?.as_str()
}

/// Minimal HTML-text escaping for the values interpolated into element text /
/// the `<title>` (not attribute context): `&`, `<`, `>`.
fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Wrap an already-serialized JSON string as a single-quoted JavaScript string
/// literal so `JSON.parse(<literal>)` rebuilds the figure object. Escapes the
/// backslashes and single quotes the JSON text may contain.
fn js_string_literal(json: &str) -> String {
    let mut out = String::with_capacity(json.len() + 2);
    out.push('\'');
    for ch in json.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

// --- Native desktop render host (gui feature only) ---------------------------

/// Open `html` in a native desktop window via [`tao`] + [`wry`] and run the
/// event loop until the window is closed.
///
/// Available only under the `gui` feature; requires a platform webview runtime
/// (`WebView2` / `WebKitGTK` / `WKWebView`). Errors surface as the `wry` / `tao`
/// error types boxed behind [`std::error::Error`].
///
/// # Errors
///
/// Returns an error if the event loop or webview cannot be created (e.g. no
/// system webview is installed).
#[cfg(feature = "gui")]
pub fn open_window(html: &str) -> Result<(), Box<dyn std::error::Error>> {
    use tao::event::{Event, WindowEvent};
    use tao::event_loop::{ControlFlow, EventLoop};
    use tao::window::WindowBuilder;
    use wry::WebViewBuilder;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("tdw-chart-host")
        .build(&event_loop)?;

    let _webview = WebViewBuilder::new().with_html(html).build(&window)?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}

/// Render `figure` to its HTML host page and [`open_window`] it in a native
/// desktop window.
///
/// Available only under the `gui` feature. The default build should call
/// [`render_html`] and host the page itself.
///
/// # Errors
///
/// Propagates any error from [`open_window`].
#[cfg(feature = "gui")]
pub fn show(figure: &Value) -> Result<(), Box<dyn std::error::Error>> {
    open_window(&render_html(figure))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A small candlestick-shaped figure, mirroring what `tdw-charting`'s
    /// `candlestick` builder emits (`{ data: [...], layout: {...} }`).
    fn candlestick_figure() -> Value {
        json!({
            "data": [{
                "type": "candlestick",
                "name": "OHLC",
                "x": ["2026-01-01", "2026-01-02"],
                "open": [10.0, 10.0],
                "high": [10.5, 11.2],
                "low": [9.8, 9.9],
                "close": [10.0, 11.0]
            }],
            "layout": {"title": "Candlestick", "template": "plotly_dark"}
        })
    }

    #[test]
    fn html_embeds_plotly_script_tag() {
        let html = render_html(&candlestick_figure());
        assert!(
            html.contains(PLOTLY_SCRIPT_SRC),
            "host page must load plotly.js"
        );
        assert!(
            html.contains("<script src=\"https://cdn.plot.ly/"),
            "plotly.js must be a <script> tag, got: {html}"
        );
    }

    #[test]
    fn html_embeds_the_figure_and_calls_new_plot() {
        let figure = candlestick_figure();
        let html = render_html(&figure);
        // The figure's distinguishing content is embedded.
        assert!(html.contains("candlestick"), "figure data must be embedded");
        assert!(html.contains("OHLC"));
        assert!(html.contains("2026-01-02"));
        // And the page calls Plotly.newPlot into the render target.
        assert!(html.contains("Plotly.newPlot"), "must call Plotly.newPlot");
        assert!(html.contains(&format!("id=\"{RENDER_TARGET_ID}\"")));
    }

    #[test]
    fn html_is_a_self_contained_document() {
        let html = render_html(&candlestick_figure());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<html"));
        assert!(html.contains("</html>"));
        assert!(html.contains("<head>") && html.contains("</head>"));
        assert!(html.contains("<body>") && html.contains("</body>"));
    }

    #[test]
    fn round_trip_embedded_json_parses_back_to_the_figure() {
        // The embedded figure JSON must round-trip: extract the single-quoted JS
        // string literal, undo our escaping, and parse it back to the figure.
        let figure = candlestick_figure();
        let html = render_html(&figure);
        let start =
            html.find("JSON.parse('").expect("has JSON.parse literal") + "JSON.parse('".len();
        let rest = &html[start..];
        let end = find_literal_end(rest).expect("closing quote");
        let literal = &rest[..end];
        let json = literal
            .replace("\\'", "'")
            .replace("\\n", "\n")
            .replace("\\r", "\r")
            .replace("\\\\", "\\")
            .replace("<\\/", "</");
        let parsed: Value = serde_json::from_str(&json).expect("embedded JSON parses");
        assert_eq!(parsed, figure, "embedded figure round-trips exactly");
    }

    /// Find the index of the unescaped closing `'` of a JS single-quoted literal.
    fn find_literal_end(s: &str) -> Option<usize> {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 2,
                b'\'' => return Some(i),
                _ => i += 1,
            }
        }
        None
    }

    #[test]
    fn title_comes_from_layout_when_present() {
        let html = render_html(&candlestick_figure());
        assert!(html.contains("<title>Candlestick</title>"));
    }

    #[test]
    fn title_defaults_when_layout_title_missing() {
        let figure = json!({"data": [], "layout": {}});
        let html = render_html(&figure);
        assert!(html.contains("<title>Chart</title>"));
    }

    #[test]
    fn script_break_out_in_figure_is_neutralized() {
        // A figure string containing `</script>` must not close the embedding
        // <script> tag in the host page.
        let figure = json!({"data": [], "layout": {"title": "x", "note": "</script><b>x"}});
        let html = render_html(&figure);
        assert!(
            !html.contains("</script><b>"),
            "raw </script> break-out must be defused"
        );
        // But it still round-trips through JSON.parse.
        assert!(html.contains("Plotly.newPlot"));
    }
}
