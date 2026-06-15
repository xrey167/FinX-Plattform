#![forbid(unsafe_code)]
//! `tdw-chart-host`: open a `tdw-charting` Plotly figure-JSON in a native
//! desktop window (openbb-ecosystem-p1 G005 — the `PyWry`-equivalent CLI).
//!
//! Reads a figure-JSON from a file path argument, or from stdin when no path is
//! given, and shows it in a native webview window.
//!
//! Built only under `--features gui` (it needs `wry` + `tao` and a platform
//! webview runtime); the crate's default build ships only the pure-Rust HTML
//! spec assembler in the library.
//!
//! Usage:
//!   `tdw-chart-host figure.json`   — read the figure from a file
//!   `cat figure.json | tdw-chart-host`   — read the figure from stdin
#![cfg(feature = "gui")]

use std::io::Read;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = match std::env::args().nth(1) {
        Some(path) => std::fs::read_to_string(&path)
            .map_err(|error| format!("failed to read figure from {path}: {error}"))?,
        None => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|error| format!("failed to read figure from stdin: {error}"))?;
            buf
        }
    };

    let figure: serde_json::Value =
        serde_json::from_str(&raw).map_err(|error| format!("figure is not valid JSON: {error}"))?;

    tdw_chart_host::show(&figure)
}
