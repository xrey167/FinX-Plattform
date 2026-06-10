//! End-to-end tests for the `OpenBB` Workspace bridge route family
//! (`workspace-route`) against an in-memory daemon.
//!
//! These drive the real `tdw_app_server::serve_workspace_http` listener with a
//! `RestApiState` wrapping `AppState::in_memory_for_tests()`, so the
//! `widget-data` happy path runs the full policy-guarded `Op::FetchData` path
//! and resolves the always-registered `fileset` fixture offline (no network or
//! credentials). The `widgets.json` / `apps.json` responses are derived from the
//! endpoint catalog. CORS preflight and the optional `X-TDW-API-KEY` auth are
//! exercised at the transport.

#![cfg(feature = "workspace-route")]

use std::time::Duration;

use tdw_app_server::{CancellationToken, WorkspaceConfig, serve_workspace_http};
use tdw_service_api::{AppState, RestApiState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Spin up a workspace listener with `config` on an ephemeral port.
async fn with_server<F, Fut>(config: WorkspaceConfig, body: F)
where
    F: FnOnce(std::net::SocketAddr) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let state = AppState::in_memory_for_tests().await;
    let handler = RestApiState::new(state).into_handler();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancel = CancellationToken::new();
    let cancel_srv = cancel.clone();
    let server = tokio::spawn(async move {
        serve_workspace_http(listener, handler, config, cancel_srv)
            .await
            .ok();
    });

    body(addr).await;

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
}

/// Send a raw request (with `extra_headers` already CRLF-terminated) and return
/// the full response bytes.
async fn raw_request(
    addr: std::net::SocketAddr,
    method: &str,
    target: &str,
    extra_headers: &str,
) -> Vec<u8> {
    let mut conn = TcpStream::connect(addr).await.expect("connect");
    let request = format!(
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{extra_headers}\r\n"
    );
    conn.write_all(request.as_bytes()).await.expect("write");
    conn.flush().await.expect("flush");
    let mut resp = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), conn.read_to_end(&mut resp)).await;
    resp
}

fn response_status(resp: &[u8]) -> u16 {
    let text = std::str::from_utf8(resp).unwrap_or("");
    text.split(' ').nth(1).unwrap_or("0").parse().unwrap_or(0)
}

fn response_headers(resp: &[u8]) -> String {
    let text = std::str::from_utf8(resp).unwrap_or("");
    text.split_once("\r\n\r\n")
        .map_or("", |(head, _)| head)
        .to_string()
}

fn response_body_json(resp: &[u8]) -> serde_json::Value {
    let text = std::str::from_utf8(resp).unwrap_or("");
    let body = text.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    serde_json::from_str(body).unwrap_or(serde_json::Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn widgets_json_parses_and_contains_equity_historical_widget() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(addr, "GET", "/widgets.json", "").await;
        assert_eq!(response_status(&resp), 200);
        let body = response_body_json(&resp);
        let widget = &body["equity_price_historical"];
        assert!(widget.is_object(), "equity historical widget present");
        assert_eq!(widget["type"], "chart");
        assert_eq!(widget["data"]["dataKey"], "results");
        assert_eq!(widget["endpoint"], "/widget-data/equity/price/historical");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apps_json_parses_and_names_the_default_app() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(addr, "GET", "/apps.json", "").await;
        assert_eq!(response_status(&resp), 200);
        let body = response_body_json(&resp);
        assert!(
            body.get("FinX Market Overview").is_some(),
            "the curated default app is served"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn widget_data_offline_fileset_happy_path() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(
            addr,
            "GET",
            "/widget-data/equity/price/historical?symbol=AAPL&provider=fileset",
            "",
        )
        .await;
        assert_eq!(
            response_status(&resp),
            200,
            "response: {}",
            String::from_utf8_lossy(&resp)
        );
        let body = response_body_json(&resp);
        // Rows live under `results` (the dataKey the widget declares).
        let results = body["results"].as_array().expect("results array");
        assert!(!results.is_empty(), "fileset fixture yields rows offline");
        assert_eq!(results[0]["symbol"], "AAPL");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_preflight_options_returns_204_with_allow_headers() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(
            addr,
            "OPTIONS",
            "/widget-data/equity/price/historical",
            "Origin: https://pro.openbb.co\r\n",
        )
        .await;
        assert_eq!(response_status(&resp), 204);
        let headers = response_headers(&resp);
        assert!(
            headers.contains("Access-Control-Allow-Origin: https://pro.openbb.co"),
            "preflight echoes the allowed origin; headers: {headers}"
        );
        assert!(
            headers.contains("X-TDW-API-KEY"),
            "preflight advertises the auth header; headers: {headers}"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_returns_401_when_key_set_and_missing() {
    let config = WorkspaceConfig {
        api_key: Some("s3cret".to_string()),
        ..WorkspaceConfig::default()
    };
    with_server(config, |addr| async move {
        // No X-TDW-API-KEY header -> 401.
        let resp = raw_request(addr, "GET", "/widgets.json", "").await;
        assert_eq!(response_status(&resp), 401);

        // Wrong key -> 401.
        let resp = raw_request(addr, "GET", "/widgets.json", "X-TDW-API-KEY: wrong\r\n").await;
        assert_eq!(response_status(&resp), 401);

        // Correct key -> 200.
        let resp = raw_request(addr, "GET", "/widgets.json", "X-TDW-API-KEY: s3cret\r\n").await;
        assert_eq!(response_status(&resp), 200);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_widget_data_route_returns_400() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(addr, "GET", "/widget-data/does/not/exist", "").await;
        assert_eq!(response_status(&resp), 400);
    })
    .await;
}
