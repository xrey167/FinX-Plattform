//! Example 20 — the real data backend: boot the in-memory daemon and serve the
//! catalog-derived `widgets.json`.
//!
//! Where example 10 hand-wrote one widget, here the whole `widgets.json` is
//! *derived* from the endpoint catalog: every `Fetch` route becomes a widget
//! (60+ of them), and the `/widget-data/...` endpoint runs the real
//! policy-guarded fetch path. It is still fully offline — `equity/price/historical`
//! resolves the always-registered `fileset` fixture, so no provider key is
//! needed.
//!
//! [`fetch_widgets_json`] and [`fetch_equity_historical`] boot the workspace
//! server, issue one request, and return the parsed document so a `main()` can
//! print it and a test can assert on it.

use serde_json::Value;
use tdw_app_server::WorkspaceConfig;

use crate::client;
use crate::server::{RunningServer, start_workspace};

/// The offline data route the example fetches — equity price history for `AAPL`
/// served by the `fileset` fixture (no network, no credentials).
pub const HISTORICAL_TARGET: &str =
    "/widget-data/equity/price/historical?symbol=AAPL&provider=fileset";

/// Boot the workspace backend (default CORS/auth) on an ephemeral port.
///
/// # Errors
///
/// Returns any error binding the listener.
pub async fn start() -> std::io::Result<RunningServer> {
    start_workspace(WorkspaceConfig::default()).await
}

/// Boot the backend, fetch `GET /widgets.json`, and return the parsed document
/// alongside the number of widgets it derived.
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_widgets_json() -> std::io::Result<(Value, usize)> {
    let server = start().await?;
    let response = client::get(server.addr(), "/widgets.json", "").await?;
    server.shutdown().await;
    let document = response.json();
    let count = document.as_object().map_or(0, serde_json::Map::len);
    Ok((document, count))
}

/// Boot the backend and fetch the offline equity-historical widget data,
/// returning the parsed envelope (rows live under `results`).
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_equity_historical() -> std::io::Result<Value> {
    let server = start().await?;
    let response = client::get(server.addr(), HISTORICAL_TARGET, "").await?;
    server.shutdown().await;
    Ok(response.json())
}
