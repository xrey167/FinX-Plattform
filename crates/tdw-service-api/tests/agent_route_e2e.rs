//! End-to-end tests for the `OpenBB` Workspace **agent protocol** bridge route
//! family (`agent-route`) against an in-memory daemon + the offline
//! `StubLanguageModel`.
//!
//! These drive the real `tdw_app_server::serve_agent_http` listener with an
//! `AgentBridgeState` (the default offline stub model), so the whole copilot
//! turn — parse `QueryRequest` → assemble prompt → drive the streaming stub →
//! write SSE — runs with no network or credentials. The `agents.json` response,
//! the streamed `POST /v1/query` transcript, the two-request widget-data round
//! trip, and the CORS/auth posture (shared with the workspace family) are all
//! exercised at the transport.

#![cfg(feature = "agent-route")]

use std::time::Duration;

use tdw_app_server::{CancellationToken, WorkspaceConfig, serve_agent_http};
use tdw_service_api::AgentBridgeState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Spin up an agent listener with `config` on an ephemeral port, advertising
/// `query_url` in `agents.json`.
async fn with_server<F, Fut>(config: WorkspaceConfig, body: F)
where
    F: FnOnce(std::net::SocketAddr) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let handler = AgentBridgeState::new().into_handler();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let query_url = format!("http://{addr}/v1/query");
    let cancel = CancellationToken::new();
    let cancel_srv = cancel.clone();
    let server = tokio::spawn(async move {
        serve_agent_http(listener, handler, config, query_url, cancel_srv)
            .await
            .ok();
    });

    body(addr).await;

    cancel.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
}

/// Send a raw request and return the full response bytes.
async fn raw_request(
    addr: std::net::SocketAddr,
    method: &str,
    target: &str,
    extra_headers: &str,
    body: &str,
) -> Vec<u8> {
    let mut conn = TcpStream::connect(addr).await.expect("connect");
    let request = format!(
        "{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n{extra_headers}\r\n{body}",
        body.len()
    );
    conn.write_all(request.as_bytes()).await.expect("write");
    conn.flush().await.expect("flush");
    let mut resp = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(3), conn.read_to_end(&mut resp)).await;
    resp
}

fn response_status(resp: &[u8]) -> u16 {
    let text = String::from_utf8_lossy(resp);
    text.split(' ').nth(1).unwrap_or("0").parse().unwrap_or(0)
}

fn response_headers(resp: &[u8]) -> String {
    let text = String::from_utf8_lossy(resp);
    text.split_once("\r\n\r\n")
        .map_or("", |(head, _)| head)
        .to_string()
}

fn response_body(resp: &[u8]) -> String {
    let text = String::from_utf8_lossy(resp);
    text.split_once("\r\n\r\n")
        .map_or(String::new(), |(_, body)| body.to_string())
}

fn response_body_json(resp: &[u8]) -> serde_json::Value {
    serde_json::from_str(&response_body(resp)).unwrap_or(serde_json::Value::Null)
}

/// Extract the ordered `event:` names from an SSE response body.
fn sse_event_names(resp: &[u8]) -> Vec<String> {
    response_body(resp)
        .lines()
        .filter_map(|line| line.strip_prefix("event: ").map(str::to_string))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_json_parses_and_advertises_the_default_copilot() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(addr, "GET", "/agents.json", "", "").await;
        assert_eq!(response_status(&resp), 200);
        let body = response_body_json(&resp);
        let object = body.as_object().expect("agents.json is an object");
        assert_eq!(object.len(), 1, "exactly one default copilot");
        let (_id, entry) = object.iter().next().expect("one entry");
        assert_eq!(entry["features"]["streaming"], true);
        assert!(
            entry["endpoints"]["query"]
                .as_str()
                .is_some_and(|url| url.ends_with("/v1/query")),
            "the query endpoint points back at the listener"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_streams_reasoning_step_then_message_chunks() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let body = serde_json::json!({
            "messages": [{"role": "human", "content": "What is a P/E ratio?"}]
        })
        .to_string();
        let resp = raw_request(addr, "POST", "/v1/query", "", &body).await;
        assert_eq!(response_status(&resp), 200);
        let headers = response_headers(&resp);
        assert!(
            headers.contains("Content-Type: text/event-stream"),
            "SSE content type; headers: {headers}"
        );
        let names = sse_event_names(&resp);
        assert_eq!(names.first().map(String::as_str), Some("reasoning_step"));
        assert!(
            names.iter().any(|name| name == "message_chunk"),
            "streams the answer; events: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name == "get_widget_data"),
            "no primary widget => no widget-data request"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_request_widget_data_round_trip() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        // Leg 1: a primary widget, no tool result => get_widget_data + close.
        let leg1 = serde_json::json!({
            "messages": [{"role": "human", "content": "How did AAPL do?"}],
            "widgets": {"primary": [{"uuid": "w-1", "params": {"symbol": "AAPL"}}]}
        })
        .to_string();
        let resp1 = raw_request(addr, "POST", "/v1/query", "", &leg1).await;
        assert_eq!(response_status(&resp1), 200);
        let names1 = sse_event_names(&resp1);
        assert!(
            names1.iter().any(|name| name == "get_widget_data"),
            "first leg requests widget data; events: {names1:?}"
        );
        assert!(
            !names1.iter().any(|name| name == "message_chunk"),
            "first leg does not answer yet"
        );

        // Leg 2: the frontend re-POSTs with a folded tool result.
        let leg2 = serde_json::json!({
            "messages": [
                {"role": "human", "content": "How did AAPL do?"},
                {"role": "tool", "content": {"rows": [{"date": "2024-01-02", "close": 185.6}]}}
            ],
            "widgets": {"primary": [{"uuid": "w-1", "params": {"symbol": "AAPL"}}]}
        })
        .to_string();
        let resp2 = raw_request(addr, "POST", "/v1/query", "", &leg2).await;
        assert_eq!(response_status(&resp2), 200);
        let names2 = sse_event_names(&resp2);
        assert!(
            names2.iter().any(|name| name == "message_chunk"),
            "second leg answers; events: {names2:?}"
        );
        assert!(
            names2.iter().any(|name| name == "citations"),
            "second leg attributes its sources; events: {names2:?}"
        );
        assert_eq!(
            names2.last().map(String::as_str),
            Some("prompt_suggestions"),
            "second leg closes with follow-up suggestions; events: {names2:?}"
        );
        // The folded tool data reaches the answer (the stub echoes context).
        let answer: String = response_body(&resp2)
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .collect();
        assert!(answer.contains("185.6"), "folded widget data is in context");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_json_advertises_configurable_feature_objects() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(addr, "GET", "/agents.json", "", "").await;
        assert_eq!(response_status(&resp), 200);
        let body = response_body_json(&resp);
        let (_id, entry) = body
            .as_object()
            .and_then(|map| map.iter().next())
            .expect("one entry");
        let features = &entry["features"];
        // The dashboard-search capability is now wired/advertised.
        assert_eq!(features["widget-dashboard-search"], true);
        // The four documented feature objects are present.
        assert_eq!(features["web-search"]["type"], "toggle");
        assert_eq!(features["deep-research"]["type"], "toggle");
        assert_eq!(features["model"]["type"], "select");
        assert_eq!(features["agent-name"]["type"], "text");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_closes_with_prompt_suggestions() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let body = serde_json::json!({
            "messages": [{"role": "human", "content": "What is a P/E ratio?"}]
        })
        .to_string();
        let resp = raw_request(addr, "POST", "/v1/query", "", &body).await;
        let names = sse_event_names(&resp);
        assert_eq!(
            names.last().map(String::as_str),
            Some("prompt_suggestions"),
            "the answered turn ends with follow-up suggestions; events: {names:?}"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_requests_multiple_primary_widgets_in_one_batch() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let body = serde_json::json!({
            "messages": [{"role": "human", "content": "compare AAPL and MSFT"}],
            "widgets": {"primary": [
                {"uuid": "w-1", "params": {"symbol": "AAPL"}},
                {"uuid": "w-2", "params": {"symbol": "MSFT"}}
            ]}
        })
        .to_string();
        let resp = raw_request(addr, "POST", "/v1/query", "", &body).await;
        assert_eq!(response_status(&resp), 200);
        // The single get_widget_data frame names both widgets.
        let data_line: String = response_body(&resp)
            .lines()
            .skip_while(|line| *line != "event: get_widget_data")
            .find_map(|line| line.strip_prefix("data: ").map(str::to_string))
            .expect("a get_widget_data frame");
        let data: serde_json::Value = serde_json::from_str(&data_line).expect("json");
        assert_eq!(
            data["data_sources"].as_array().map(Vec::len),
            Some(2),
            "both primary widgets requested at once: {data}"
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cors_preflight_options_returns_204() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(
            addr,
            "OPTIONS",
            "/v1/query",
            "Origin: https://pro.openbb.co\r\n",
            "",
        )
        .await;
        assert_eq!(response_status(&resp), 204);
        let headers = response_headers(&resp);
        assert!(headers.contains("Access-Control-Allow-Origin: https://pro.openbb.co"));
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
        let resp = raw_request(addr, "GET", "/agents.json", "", "").await;
        assert_eq!(response_status(&resp), 401);

        let resp = raw_request(addr, "GET", "/agents.json", "X-TDW-API-KEY: s3cret\r\n", "").await;
        assert_eq!(response_status(&resp), 200);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_query_body_returns_400() {
    with_server(WorkspaceConfig::default(), |addr| async move {
        let resp = raw_request(addr, "POST", "/v1/query", "", "not json").await;
        assert_eq!(response_status(&resp), 400);
    })
    .await;
}
