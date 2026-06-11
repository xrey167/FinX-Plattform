//! Example 10 — the raw `OpenBB` Workspace data-backend contract, hand-rolled.
//!
//! A Workspace data backend is just two things over HTTP: a `widgets.json`
//! manifest describing the widgets, and one data endpoint per widget that
//! returns the rows under the widget's declared `dataKey`. This example writes
//! both *by hand* — one widget, one endpoint — over a tiny standalone HTTP server
//! built from `tokio` primitives, so the contract is visible with no framework
//! and no daemon in the way. Examples 20+ replace this hand-work with the real
//! catalog-derived backend.
//!
//! The two documents are pure values ([`widgets_document`] / [`widget_data`]) so
//! a test can assert them directly; [`serve`] wraps them in the smallest HTTP
//! server that answers the two routes (plus a 404 for anything else).

use std::net::SocketAddr;

use serde_json::{Value, json};
use tdw_app_server::CancellationToken;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The data-endpoint path the single hand-written widget points at.
pub const DATA_ENDPOINT: &str = "/widget-data/example/price";

/// The hand-written `widgets.json`: a JSON object keyed by widget id, holding a
/// single price-history chart widget. Field names match the public Workspace
/// `widgets.json` reference exactly.
#[must_use]
pub fn widgets_document() -> Value {
    json!({
        "example_price": {
            "name": "Example Price History",
            "description": "A hand-written single-widget backend: daily closes.",
            "category": "example",
            "type": "chart",
            "endpoint": DATA_ENDPOINT,
            "gridData": { "w": 20, "h": 12 },
            "params": [
                { "paramName": "symbol", "type": "text", "value": "DEMO", "label": "Symbol" }
            ],
            "data": {
                "dataKey": "results",
                "chart": { "chartType": "line" }
            }
        }
    })
}

/// The hand-written data-endpoint response: rows live under the `results` key the
/// widget declares as its `dataKey`. Static, offline, deterministic.
#[must_use]
pub fn widget_data() -> Value {
    json!({
        "results": [
            { "date": "2024-01-02", "close": 184.25 },
            { "date": "2024-01-03", "close": 185.60 },
            { "date": "2024-01-04", "close": 183.95 }
        ]
    })
}

/// A booted minimal backend: its address plus the cancel/join handles.
#[derive(Debug)]
pub struct MinimalServer {
    addr: SocketAddr,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl MinimalServer {
    /// The loopback address the server is listening on.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The base URL (`http://<addr>`) clients should target.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Signal the accept loop to stop and await the task.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }

    /// Block until the server task finishes.
    ///
    /// # Errors
    ///
    /// Returns the join error if the server task panicked.
    pub async fn join(self) -> Result<(), tokio::task::JoinError> {
        self.task.await
    }
}

/// Boot the hand-rolled backend on an ephemeral loopback port.
///
/// Serves exactly `GET /widgets.json` and `GET /widget-data/example/price`;
/// everything else gets a `404`. The whole server is a few dozen lines so the
/// raw request/response shape is teachable.
///
/// # Errors
///
/// Returns any error binding the ephemeral listener.
pub async fn serve() -> std::io::Result<MinimalServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let cancel = CancellationToken::new();
    let cancel_srv = cancel.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                () = cancel_srv.cancelled() => break,
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, _peer)) => {
                            tokio::spawn(handle_conn(stream));
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });
    Ok(MinimalServer { addr, cancel, task })
}

/// Answer one connection: read the request line, route on the target, write the
/// matching JSON document (or a 404).
async fn handle_conn(mut stream: TcpStream) {
    let mut buffer = [0u8; 1024];
    let read = match stream.read(&mut buffer).await {
        Ok(0) | Err(_) => return,
        Ok(n) => n,
    };
    let request = String::from_utf8_lossy(&buffer[..read]);
    let target = request
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("");

    let response = match target {
        "/widgets.json" => json_response(200, &widgets_document()),
        DATA_ENDPOINT => json_response(200, &widget_data()),
        _ => json_response(404, &json!({ "error": "not found" })),
    };
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Render a minimal HTTP/1.1 JSON response with `Connection: close`.
fn json_response(status: u16, body: &Value) -> String {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let encoded = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{encoded}",
        encoded.len()
    )
}
