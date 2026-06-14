# tdw-chart-host Readiness Worksheet

Generated during the openbb-ecosystem-p1 **G005** landing (the desktop
chart-render host — the PyWry-equivalent). `tdw-charting` emits the Plotly figure
*spec*; this crate is the render *host* that turns that spec into a viewable
page, and — behind an optional, non-default feature — a native desktop window.

## Evidence Snapshot

- Manifest: `crates/tdw-chart-host/Cargo.toml`.
- Targets: lib, bin (`tdw-chart-host`, `required-features = ["gui"]`).
- Local deps: none (library); `serde_json` only.
- Optional deps: `tao` + `wry` — **optional**, behind the non-default `gui`
  feature (`gui = ["dep:wry", "dep:tao"]`).
- Dev deps: `tdw-charting`, `tdw-domain` (the integration round-trip feeds a real
  charting candlestick figure through the HTML assembler).
- Reverse deps: none yet (the host is a leaf; the daemon already ships the figure
  spec via `tdw-charting`, and a desktop/CLI consumer opts into this host).
- Features: `default = []` (pure-Rust HTML assembler only); `gui` (native window).
- Tests: HTML-assembler unit tests (embeds plotly.js `<script>`, embeds the
  figure + `Plotly.newPlot`, self-contained document, JSON round-trip, title
  derivation, `</script>` break-out defusing) plus a `tdw-charting` candlestick
  round-trip integration test. All run under the **default** feature with NO
  native webview dependency.
- Docs/examples: this worksheet plus module-level docs that record the `gui`
  feature + native-webview boundary and the plotly.js host-page clean-room note.

## Native-dependency boundary (CI-safety)

`wry` + `tao` bind the host platform's **native webview** (WebView2 on Windows,
WebKitGTK on Linux, `WKWebView` on macOS), which a CI runner may not have. They
are therefore declared `optional = true` and gated behind the **non-default**
`gui` feature, and every line that uses them is guarded by
`#[cfg(feature = "gui")]` (the `show` / `open_window` fns and the
`src/bin/tdw-chart-host.rs` binary, which also carries `required-features`).

Consequently the DEFAULT workspace build — what `cargo build`,
`cargo clippy --workspace --all-targets`, and `cargo test --workspace` run —
compiles **only** the pure-Rust HTML spec assembler and never pulls wry/tao. Core
CI stays green on a runner with no system webview library. The desktop window
requires `--features gui` **and** a platform webview at build+run time; lint it
explicitly with `cargo clippy -p tdw-chart-host --features gui`.

## Release Assessment

- Default layer (`render_html`) is a pure, offline, deterministic function: it
  wraps a Plotly figure JSON in a self-contained HTML page that loads plotly.js
  via a `<script>` tag, embeds the figure as a JS string literal, and calls
  `Plotly.newPlot(target, figure.data, figure.layout)`. ZERO native dependency.
- Embedding safety: the serialized figure is embedded as a single-quoted JS
  string literal parsed by `JSON.parse`, and any `</` is rewritten to `<\/` so a
  figure containing `</script>` cannot break out of the host page's script tag.
  A round-trip test proves the embedded figure parses back identically.
- `gui` layer (`show` / `open_window`) opens that HTML in a native window via a
  `tao` event loop + a `wry` `WebViewBuilder` (`with_html`), running until the
  window is closed — the PyWry pattern (PyWry wraps Rust `wry`).
- Clean-room: the host page is the documented plotly.js getting-started shape (a
  render `<div>` + `Plotly.newPlot`), cited to `plotly.com/javascript` in the
  module docs. No reference implementation was consulted.

## Verdict

Ready with follow-ups. The spec-assembler core (default feature) is complete with
unit + integration tests and is CI-safe. The native window (`gui`) is a thin
`tao`+`wry` host; richer host features (multi-figure tabs, export-to-PNG via the
webview, a persistent window server) are intentionally out of scope for G005 and
are a later append.
