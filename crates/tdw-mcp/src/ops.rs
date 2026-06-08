//! Blocking ops surface for `tdw-mcp --streamable-http`: `/health`, `/ready`,
//! `/metrics` on a separate listener, plus a process-wide graceful-shutdown
//! flag.
//!
//! `tdw-mcp` serves its Streamable HTTP transport on a dedicated OS thread with
//! no `tokio` runtime, so this module mirrors that model with a small blocking
//! `TcpListener` poll loop rather than the async `serve_ops` used by the daemon
//! and worker. The Prometheus rendering and response classification are reused
//! from [`tdw_app_server::ops`].

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tdw_app_server::ops::{OpsResponse, OpsRoute, classify_route};

use crate::McpMetrics;

/// Process-wide graceful-shutdown flag.
///
/// Set by the signal handler ([`install_signal_handler`]); polled by the
/// Streamable HTTP accept loop and the ops listener so both stop accepting new
/// connections and exit. Cloneable (`Arc`) across the serve thread and the ops
/// thread.
#[derive(Clone, Default)]
pub struct Shutdown {
    flag: Arc<AtomicBool>,
}

impl Shutdown {
    /// A fresh, un-triggered shutdown handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Trigger shutdown (idempotent).
    pub fn trigger(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Whether shutdown has been requested.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Install a SIGTERM (unix) + Ctrl-C signal handler that sets `shutdown`.
///
/// Uses `tokio::signal` from a tiny dedicated current-thread runtime, so it
/// works without the MCP serve loop owning a runtime. Best-effort: a failure to
/// build the runtime or install the handler is logged and the process simply
/// keeps relying on listener close / stdio EOF for shutdown.
pub fn install_signal_handler(shutdown: Shutdown) {
    std::thread::Builder::new()
        .name("tdw-mcp-signal".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("tdw-mcp: signal handler runtime build failed: {error}");
                    return;
                }
            };
            runtime.block_on(async {
                tdw_app_server::shutdown_signal().await;
            });
            eprintln!("tdw-mcp: shutdown signal received; draining");
            shutdown.trigger();
        })
        .ok();
}

/// Readiness probe for the MCP ops `/ready` endpoint.
///
/// When the MCP is daemon-routed (a TCP daemon address is configured), readiness
/// is "daemon reachable": a bounded TCP connect to that address. When no daemon
/// address is configured (offline/stdio-style tools only), the MCP is always
/// ready once serving.
#[derive(Clone)]
pub struct McpReadiness {
    daemon_tcp_addr: Option<String>,
}

impl McpReadiness {
    /// Build a readiness probe. `daemon_tcp_addr` is the daemon's `host:port`
    /// when daemon-routed over TCP, else `None`.
    #[must_use]
    pub const fn new(daemon_tcp_addr: Option<String>) -> Self {
        Self { daemon_tcp_addr }
    }

    /// `(ready, detail)` for the `/ready` body.
    #[must_use]
    pub fn check(&self) -> (bool, String) {
        let Some(addr) = self.daemon_tcp_addr.as_deref() else {
            return (
                true,
                "ready: no daemon dependency (offline tools)\n".to_string(),
            );
        };
        addr.to_socket_addr().map_or_else(
            || (false, format!("not ready: invalid daemon address {addr}\n")),
            |socket_addr| match TcpStream::connect_timeout(&socket_addr, Duration::from_millis(500))
            {
                Ok(_) => (true, format!("ready: daemon reachable at {addr}\n")),
                Err(error) => (
                    false,
                    format!("not ready: daemon unreachable at {addr}: {error}\n"),
                ),
            },
        )
    }
}

/// Tiny helper so a `host:port` string yields a `SocketAddr` without pulling in
/// extra deps.
trait ToSocketAddr {
    fn to_socket_addr(&self) -> Option<std::net::SocketAddr>;
}

impl ToSocketAddr for str {
    fn to_socket_addr(&self) -> Option<std::net::SocketAddr> {
        use std::net::ToSocketAddrs;
        self.to_socket_addrs().ok().and_then(|mut iter| iter.next())
    }
}

/// Serve `/health`, `/ready`, `/metrics` on `bind` until `shutdown` triggers.
///
/// Blocking poll loop: the listener is non-blocking, accept is retried on a
/// short interval, and the loop exits when `shutdown.is_triggered()`. One
/// short-lived connection per request.
///
/// # Errors
///
/// Returns an `io::Error` if the listener cannot be bound or put into
/// non-blocking mode. Per-connection errors are swallowed.
pub fn serve_ops_blocking(
    bind: &str,
    metrics: McpMetrics,
    readiness: McpReadiness,
    shutdown: Shutdown,
) -> std::io::Result<()> {
    let listener = TcpListener::bind(bind)?;
    listener.set_nonblocking(true)?;
    eprintln!("tdw-mcp ops listener on http://{bind} (/health /ready /metrics)");

    while !shutdown.is_triggered() {
        match listener.accept() {
            Ok((stream, _peer)) => {
                handle_conn(stream, &metrics, &readiness);
            }
            Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Ok(())
}

fn handle_conn(mut stream: TcpStream, metrics: &McpMetrics, readiness: &McpReadiness) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let Some((method, target)) = read_request_line(&mut stream) else {
        return;
    };
    let response = match classify_route(&method, &target) {
        OpsRoute::Health => OpsResponse::health_ok(),
        OpsRoute::Ready => {
            let (ready, detail) = readiness.check();
            OpsResponse::ready(ready, detail)
        }
        OpsRoute::Metrics => OpsResponse::metrics(metrics.render()),
        OpsRoute::NotFound => OpsResponse::not_found(),
    };
    let _ = stream.write_all(response.to_http_string().as_bytes());
    let _ = stream.flush();
}

const MAX_OPS_HEADER_BYTES: usize = 8 * 1024;

/// Read up to the first `\r\n` and return `(method, target)`. `None` on EOF,
/// error, oversized input, or a malformed request line.
fn read_request_line(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut buf = Vec::with_capacity(256);
    let mut scratch = [0u8; 256];
    loop {
        if buf.len() > MAX_OPS_HEADER_BYTES {
            return None;
        }
        let n = stream.read(&mut scratch).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&scratch[..n]);
        if let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
            let line = std::str::from_utf8(&buf[..pos]).ok()?;
            let mut parts = line.split_ascii_whitespace();
            let method = parts.next()?.to_string();
            let target = parts.next()?.to_string();
            return Some((method, target));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_flag_round_trips() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_triggered());
        shutdown.trigger();
        assert!(shutdown.is_triggered());
    }

    #[test]
    fn readiness_without_daemon_is_ready() {
        let readiness = McpReadiness::new(None);
        let (ready, detail) = readiness.check();
        assert!(ready, "detail: {detail}");
        assert!(detail.contains("no daemon dependency"), "detail: {detail}");
    }

    #[test]
    fn readiness_with_unreachable_daemon_is_not_ready() {
        // Bind then drop to get a definitely-closed local port.
        let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let addr = probe.local_addr().expect("probe addr").to_string();
        drop(probe);

        let readiness = McpReadiness::new(Some(addr));
        let (ready, detail) = readiness.check();
        assert!(!ready, "detail: {detail}");
        assert!(detail.contains("not ready"), "detail: {detail}");
    }

    #[test]
    fn metrics_render_counts_methods() {
        let metrics = McpMetrics::new();
        metrics.record("tools/list");
        metrics.record("tools/call");
        metrics.record("tools/call");
        let body = metrics.render();
        assert!(
            body.contains("tdw_mcp_requests_total{method=\"tools/list\"} 1"),
            "got: {body}"
        );
        assert!(
            body.contains("tdw_mcp_requests_total{method=\"tools/call\"} 2"),
            "got: {body}"
        );
    }

    #[test]
    fn metrics_render_empty_still_emits_family_header() {
        let body = McpMetrics::new().render();
        assert!(
            body.contains("# TYPE tdw_mcp_requests_total counter"),
            "got: {body}"
        );
    }
}
