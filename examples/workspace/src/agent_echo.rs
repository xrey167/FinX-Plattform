//! Example 40 — the minimal copilot: `agents.json` + a streamed answer.
//!
//! A Workspace copilot is an HTTP backend that publishes `agents.json` (who the
//! copilot is and where to query it) and answers `POST /v1/query` with a stream
//! of Server-Sent Events. This example boots the agent bridge backed by the
//! offline `StubLanguageModel` and runs the simplest turn: a question with no
//! widgets attached streams an opening `reasoning_step` and then the answer as
//! `message_chunk`s — no network, no credentials.
//!
//! [`fetch_agents_json`] returns the manifest; [`ask`] runs one no-widget turn
//! and returns the parsed [`crate::client::Response`] so a test can assert the
//! SSE event order.

use serde_json::{Value, json};
use tdw_app_server::WorkspaceConfig;

use crate::client::{self, Response};
use crate::server::{RunningServer, start_agent};

/// Boot the agent (copilot) bridge with default CORS/auth on an ephemeral port.
///
/// # Errors
///
/// Returns any error binding the listener.
pub async fn start() -> std::io::Result<RunningServer> {
    start_agent(WorkspaceConfig::default()).await
}

/// Boot the bridge and fetch the `agents.json` manifest it advertises.
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn fetch_agents_json() -> std::io::Result<Value> {
    let server = start().await?;
    let response = client::get(server.addr(), "/agents.json", "").await?;
    server.shutdown().await;
    Ok(response.json())
}

/// Build the `POST /v1/query` body for a single no-widget question.
#[must_use]
pub fn question_body(prompt: &str) -> String {
    json!({ "messages": [{ "role": "human", "content": prompt }] }).to_string()
}

/// Boot the bridge and ask one no-widget question, returning the SSE response.
///
/// # Errors
///
/// Returns any error booting the server or issuing the request.
pub async fn ask(prompt: &str) -> std::io::Result<Response> {
    let server = start().await?;
    let body = question_body(prompt);
    let response = client::post(server.addr(), "/v1/query", &body, "").await?;
    server.shutdown().await;
    Ok(response)
}
