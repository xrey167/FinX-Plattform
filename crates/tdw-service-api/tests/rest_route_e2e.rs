//! End-to-end tests for the catalog-derived REST route family
//! (`rest-api-route`) against an in-memory daemon.
//!
//! These drive the real `tdw_app_server::serve_rest_http` listener with a
//! `RestApiState` wrapping `AppState::in_memory_for_tests()`, so the full
//! policy-guarded `Op::FetchData` path runs — exactly what a client hitting the
//! daemon's REST surface exercises. Everything is offline: the happy path
//! resolves the always-registered `fileset` fixture for
//! `equity/price/historical`, so no network or credentials are touched.

#![cfg(feature = "rest-api-route")]

use std::time::Duration;

use tdw_app_server::{CancellationToken, serve_rest_http};
use tdw_service_api::{AppState, RestApiState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Spin up a REST listener on an ephemeral port and run `body(addr)`.
async fn with_server<F, Fut>(body: F)
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
        serve_rest_http(listener, handler, cancel_srv).await.ok();
    });

    body(addr).await;

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
}

/// Send a raw `GET <target>` request and return the full response bytes.
async fn raw_get(addr: std::net::SocketAddr, target: &str) -> Vec<u8> {
    let mut conn = TcpStream::connect(addr).await.expect("connect");
    let request = format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
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

fn response_body_json(resp: &[u8]) -> serde_json::Value {
    let text = std::str::from_utf8(resp).unwrap_or("");
    let body = text.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    serde_json::from_str(body).unwrap_or(serde_json::Value::Null)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn happy_path_fileset_equity_historical_returns_envelope() {
    with_server(|addr| async move {
        let resp = raw_get(
            addr,
            "/api/v1/equity/price/historical?symbol=AAPL&provider=fileset",
        )
        .await;
        assert_eq!(
            response_status(&resp),
            200,
            "response: {}",
            String::from_utf8_lossy(&resp)
        );
        let body = response_body_json(&resp);
        // ResultEnvelope shape: id / results / provider / extra.route.
        assert_eq!(body["provider"], "fileset");
        assert_eq!(body["extra"]["route"], "equity/price/historical");
        let results = body["results"].as_array().expect("results array");
        assert!(!results.is_empty(), "fileset fixture yields rows offline");
        assert_eq!(results[0]["symbol"], "AAPL");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chart_true_attaches_candlestick_spec_offline() {
    with_server(|addr| async move {
        let resp = raw_get(
            addr,
            "/api/v1/equity/price/historical?symbol=AAPL&provider=fileset&chart=true",
        )
        .await;
        assert_eq!(
            response_status(&resp),
            200,
            "response: {}",
            String::from_utf8_lossy(&resp)
        );
        let body = response_body_json(&resp);
        // The envelope carries a Plotly figure under `chart` with a candlestick
        // trace built from the OHLCV rows the fileset fixture returns.
        let chart = &body["chart"];
        assert!(chart.is_object(), "chart spec present: {body}");
        let traces = chart["data"].as_array().expect("figure data array");
        assert!(
            traces.iter().any(|t| t["type"] == "candlestick"),
            "candlestick trace present: {chart}"
        );
        assert!(chart["layout"].is_object(), "figure layout present");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chart_omitted_leaves_envelope_chartless() {
    with_server(|addr| async move {
        let resp = raw_get(
            addr,
            "/api/v1/equity/price/historical?symbol=AAPL&provider=fileset",
        )
        .await;
        assert_eq!(response_status(&resp), 200);
        let body = response_body_json(&resp);
        // No `chart=true` => the slot is skipped entirely (byte-identical to the
        // pre-chart envelope shape).
        assert!(body.get("chart").is_none(), "no chart slot: {body}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_route_returns_400_with_known_routes() {
    with_server(|addr| async move {
        let resp = raw_get(addr, "/api/v1/does/not/exist?symbol=AAPL").await;
        assert_eq!(response_status(&resp), 400);
        let body = response_body_json(&resp);
        let error = body["error"].as_str().expect("error message");
        assert!(error.contains("unknown catalog route"), "got: {error}");
        // The known-routes list is included so clients can discover valid routes.
        assert!(error.contains("equity/price/historical"), "got: {error}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_param_returns_400() {
    with_server(|addr| async move {
        // `interval=bogus` fails StandardParams validation (a caller error),
        // which the handler classifies as InvalidParams -> HTTP 400 (no
        // provider fallback).
        let resp = raw_get(
            addr,
            "/api/v1/equity/price/historical?symbol=AAPL&provider=fileset&interval=bogus",
        )
        .await;
        assert_eq!(
            response_status(&resp),
            400,
            "response: {}",
            String::from_utf8_lossy(&resp)
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openapi_json_is_served_and_parses() {
    with_server(|addr| async move {
        let resp = raw_get(addr, "/openapi.json").await;
        assert_eq!(response_status(&resp), 200);
        let body = response_body_json(&resp);
        assert_eq!(body["openapi"], "3.1.0");
        assert!(
            body["paths"]["/api/v1/equity/price/historical"]["get"].is_object(),
            "the seeded equity route is documented"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_catalog_path_returns_404() {
    with_server(|addr| async move {
        let resp = raw_get(addr, "/not/the/api").await;
        assert_eq!(response_status(&resp), 404);
    })
    .await;
}
