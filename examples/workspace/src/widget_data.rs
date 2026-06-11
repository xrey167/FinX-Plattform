//! Example 60 — the stateless two-request `get_widget_data` round trip.
//!
//! When a question needs data from a widget, the copilot cannot fetch it
//! itself: it asks the *frontend* to. Leg 1 — the copilot emits a
//! `get_widget_data` event naming the widget and closes the stream. The frontend
//! fetches that widget's data and re-POSTs the same conversation with the rows
//! folded in as a `tool` message. Leg 2 — the copilot streams the grounded
//! answer and closes with `citations`. This example scripts both legs end to end
//! over HTTP against the offline agent bridge.
//!
//! [`run`] boots one server and drives both legs against it, returning the two
//! responses so a test can assert the leg-1 `get_widget_data` request and the
//! leg-2 grounded answer + citations.

use serde_json::{Value, json};

use crate::client::{self, Response};
use crate::server::start_agent;
use tdw_app_server::WorkspaceConfig;

/// The widget the scripted conversation attaches and fetches.
pub const WIDGET_UUID: &str = "w-1";

/// The rows the "frontend" folds back in on leg 2 (a stand-in for the data it
/// would fetch from the widget's endpoint).
#[must_use]
pub fn fetched_rows() -> Value {
    json!({ "rows": [{ "date": "2024-01-02", "close": 185.6 }] })
}

/// The leg-1 request body: a question about a primary widget, no tool result.
#[must_use]
pub fn leg1_body() -> String {
    json!({
        "messages": [{ "role": "human", "content": "How did AAPL do?" }],
        "widgets": { "primary": [{ "uuid": WIDGET_UUID, "params": { "symbol": "AAPL" } }] }
    })
    .to_string()
}

/// The leg-2 request body: the same conversation with the fetched rows folded in
/// as a `tool` message.
#[must_use]
pub fn leg2_body() -> String {
    json!({
        "messages": [
            { "role": "human", "content": "How did AAPL do?" },
            { "role": "tool", "content": fetched_rows() }
        ],
        "widgets": { "primary": [{ "uuid": WIDGET_UUID, "params": { "symbol": "AAPL" } }] }
    })
    .to_string()
}

/// The two SSE responses of a scripted round trip: leg 1 (the widget-data
/// request) and leg 2 (the grounded answer).
#[derive(Clone, Debug)]
pub struct RoundTrip {
    /// The leg-1 response: emits `get_widget_data` and closes.
    pub leg1: Response,
    /// The leg-2 response: streams the answer and closes with `citations`.
    pub leg2: Response,
}

/// Boot the agent bridge and drive both legs of the round trip against it.
///
/// # Errors
///
/// Returns any error booting the server or issuing either request.
pub async fn run() -> std::io::Result<RoundTrip> {
    let server = start_agent(WorkspaceConfig::default()).await?;
    let leg1 = client::post(server.addr(), "/v1/query", &leg1_body(), "").await?;
    let leg2 = client::post(server.addr(), "/v1/query", &leg2_body(), "").await?;
    server.shutdown().await;
    Ok(RoundTrip { leg1, leg2 })
}
