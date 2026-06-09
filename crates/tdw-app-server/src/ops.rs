//! Shared service-operability surface: `/health`, `/ready`, `/metrics`.
//!
//! This module hand-rolls a tiny Prometheus text-exposition renderer and a
//! minimal HTTP/1.1 ops listener so the three long-running TDW services (the
//! daemon, `tdw-worker --serve`, and `tdw-mcp --streamable-http`) can expose
//! liveness, readiness, and metrics without pulling in a metrics framework.
//!
//! The renderer ([`render_prometheus`]) and the response classifier
//! ([`OpsResponse`]) are pure and unit-tested; the async listener
//! ([`serve_ops`]) wires them onto a `tokio` `TcpListener`. Services that run on
//! their own (non-`tokio`) thread model — e.g. `tdw-mcp`'s blocking accept loop —
//! reuse the pure pieces and supply their own transport.
//!
//! Endpoint contract:
//! * `GET /health`  → always `200` once serving (process liveness).
//! * `GET /ready`   → `200` when dependencies are reachable, else `503`.
//! * `GET /metrics` → `200` Prometheus text exposition.
//! * anything else  → `404`.

use std::fmt::Write as _;

use crate::EventMsg;

/// One Prometheus sample: a metric family name, HELP/TYPE metadata, and a value.
///
/// Only the two metric kinds the TDW services need are modelled
/// ([`MetricKind::Counter`], [`MetricKind::Gauge`]); labels are rendered inline.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    /// Metric family name, e.g. `tdw_worker_jobs`.
    pub name: String,
    /// One-line HELP text.
    pub help: String,
    /// Counter or gauge.
    pub kind: MetricKind,
    /// Label pairs rendered as `{k="v",...}` (empty for an unlabelled sample).
    pub labels: Vec<(String, String)>,
    /// The sample value.
    pub value: f64,
}

/// Prometheus metric type for the `# TYPE` line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricKind {
    /// Monotonically increasing total.
    Counter,
    /// Instantaneous value that can go up or down.
    Gauge,
}

impl MetricKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
        }
    }
}

impl Metric {
    /// Build a counter sample.
    #[must_use]
    pub fn counter(name: impl Into<String>, help: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            help: help.into(),
            kind: MetricKind::Counter,
            labels: Vec::new(),
            value,
        }
    }

    /// Build a gauge sample.
    #[must_use]
    pub fn gauge(name: impl Into<String>, help: impl Into<String>, value: f64) -> Self {
        Self {
            name: name.into(),
            help: help.into(),
            kind: MetricKind::Gauge,
            labels: Vec::new(),
            value,
        }
    }

    /// Attach a label pair, returning `self` for chaining.
    #[must_use]
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.push((key.into(), value.into()));
        self
    }
}

/// Render metrics as a Prometheus 0.0.4 text exposition body.
///
/// Consecutive samples that share a family name emit a single `# HELP`/`# TYPE`
/// header (from the first sample of the family), then one value line each. Label
/// values are escaped per the exposition format (`\\`, `"`, `\n`).
#[must_use]
pub fn render_prometheus(metrics: &[Metric]) -> String {
    let mut out = String::new();
    let mut last_family: Option<&str> = None;
    for metric in metrics {
        if last_family != Some(metric.name.as_str()) {
            let _ = writeln!(out, "# HELP {} {}", metric.name, metric.help);
            let _ = writeln!(out, "# TYPE {} {}", metric.name, metric.kind.as_str());
            last_family = Some(metric.name.as_str());
        }
        out.push_str(&metric.name);
        if !metric.labels.is_empty() {
            out.push('{');
            for (index, (key, value)) in metric.labels.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{key}=\"{}\"", escape_label_value(value));
            }
            out.push('}');
        }
        let _ = writeln!(out, " {}", format_value(metric.value));
    }
    out
}

/// Format a metric value: integers render without a decimal point, otherwise the
/// default `f64` formatting is used.
// value is finite and whole (guarded below); the f64->i64 cast is display-only
// metric formatting where a saturating cast on out-of-range values is acceptable.
#[allow(clippy::cast_possible_truncation)]
fn format_value(value: f64) -> String {
    if value.fract() == 0.0 && value.is_finite() {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Escape a Prometheus label value (`\\`, `"`, newline).
fn escape_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// The classified result of an ops request, ready to be written as an HTTP/1.1
/// response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpsResponse {
    /// HTTP status code.
    pub status: u16,
    /// Reason phrase.
    pub reason: &'static str,
    /// `Content-Type` header value.
    pub content_type: &'static str,
    /// Response body.
    pub body: String,
}

impl OpsResponse {
    /// `200 OK` plain-text liveness response.
    #[must_use]
    pub fn health_ok() -> Self {
        Self {
            status: 200,
            reason: "OK",
            content_type: "text/plain; charset=utf-8",
            body: "ok\n".to_string(),
        }
    }

    /// Readiness response: `200` when `ready`, else `503`. `detail` is the body.
    #[must_use]
    pub fn ready(ready: bool, detail: impl Into<String>) -> Self {
        if ready {
            Self {
                status: 200,
                reason: "OK",
                content_type: "text/plain; charset=utf-8",
                body: detail.into(),
            }
        } else {
            Self {
                status: 503,
                reason: "Service Unavailable",
                content_type: "text/plain; charset=utf-8",
                body: detail.into(),
            }
        }
    }

    /// `200 OK` Prometheus metrics response.
    #[must_use]
    pub const fn metrics(body: String) -> Self {
        Self {
            status: 200,
            reason: "OK",
            content_type: "text/plain; version=0.0.4; charset=utf-8",
            body,
        }
    }

    /// `404 Not Found`.
    #[must_use]
    pub fn not_found() -> Self {
        Self {
            status: 404,
            reason: "Not Found",
            content_type: "text/plain; charset=utf-8",
            body: "not found\n".to_string(),
        }
    }

    /// Serialise to an HTTP/1.1 response with `Connection: close`.
    #[must_use]
    pub fn to_http_string(&self) -> String {
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.status,
            self.reason,
            self.content_type,
            self.body.len(),
            self.body
        )
    }
}

/// The three ops routes plus a catch-all, parsed from a request target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpsRoute {
    /// `GET /health`.
    Health,
    /// `GET /ready`.
    Ready,
    /// `GET /metrics`.
    Metrics,
    /// Any other method/path.
    NotFound,
}

/// Classify an HTTP method + request target into an [`OpsRoute`]. The query
/// string (if any) is ignored. Only `GET` is accepted for the ops routes.
#[must_use]
pub fn classify_route(method: &str, target: &str) -> OpsRoute {
    if method != "GET" {
        return OpsRoute::NotFound;
    }
    let path = target.split('?').next().unwrap_or(target);
    match path {
        "/health" | "/healthz" => OpsRoute::Health,
        "/ready" | "/readyz" => OpsRoute::Ready,
        "/metrics" => OpsRoute::Metrics,
        _ => OpsRoute::NotFound,
    }
}

// ---------------------------------------------------------------------------
// Daemon dispatch metrics
// ---------------------------------------------------------------------------

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// Cheap, cloneable counters for the daemon's dispatch surface.
///
/// Tracks terminal outcome totals plus an in-flight gauge. Shared (`Arc`)
/// between the `ServiceLoop` (which records each dispatch) and the daemon's
/// `/metrics` listener (which renders a snapshot). All updates are `Relaxed`
/// atomics — the values are monitoring signals, not synchronization points.
#[derive(Clone, Default)]
pub struct DaemonMetrics {
    inner: Arc<DaemonMetricsInner>,
}

#[derive(Default)]
struct DaemonMetricsInner {
    completed: AtomicU64,
    failed: AtomicU64,
    cancelled: AtomicU64,
    in_flight: AtomicI64,
}

impl DaemonMetrics {
    /// A fresh metrics handle with all counters at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a dispatch as started (in-flight + 1).
    pub fn dispatch_started(&self) {
        self.inner.in_flight.fetch_add(1, Ordering::Relaxed);
    }

    /// Mark a dispatch as finished (in-flight - 1) and bump the matching
    /// terminal-outcome counter inferred from `events`. The last terminal event
    /// wins; a dispatch with no terminal event bumps nothing but still clears
    /// in-flight.
    pub fn dispatch_finished(&self, events: &[EventMsg]) {
        self.inner.in_flight.fetch_sub(1, Ordering::Relaxed);
        for event in events.iter().rev() {
            match event {
                EventMsg::Completed { .. } => {
                    self.inner.completed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                EventMsg::Failed { .. } => {
                    self.inner.failed.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                EventMsg::Cancelled { .. } => {
                    self.inner.cancelled.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                _ => {}
            }
        }
    }

    /// Render the dispatch metrics as a Prometheus body: a labelled
    /// `tdw_daemon_dispatch_total{outcome=...}` counter family plus a
    /// `tdw_daemon_dispatch_in_flight` gauge.
    #[must_use]
    pub fn render(&self) -> String {
        #[allow(clippy::cast_precision_loss)]
        let dispatch = |outcome: &str, value: u64| {
            Metric::counter(
                "tdw_daemon_dispatch_total",
                "Daemon dispatches by terminal outcome",
                value as f64,
            )
            .with_label("outcome", outcome)
        };
        let metrics = vec![
            dispatch("completed", self.inner.completed.load(Ordering::Relaxed)),
            dispatch("failed", self.inner.failed.load(Ordering::Relaxed)),
            dispatch("cancelled", self.inner.cancelled.load(Ordering::Relaxed)),
            Metric::gauge(
                "tdw_daemon_dispatch_in_flight",
                "Daemon dispatches currently in flight",
                #[allow(clippy::cast_precision_loss)]
                {
                    self.inner.in_flight.load(Ordering::Relaxed) as f64
                },
            ),
        ];
        render_prometheus(&metrics)
    }
}

// ---------------------------------------------------------------------------
// Async listener
// ---------------------------------------------------------------------------

// The listener uses `tokio::net`, gated out by tokio under `--cfg loom`.
#[cfg(not(loom))]
mod listener {
    use super::{OpsResponse, OpsRoute, classify_route};
    use std::future::Future;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_util::sync::CancellationToken;

    const MAX_OPS_HEADER_BYTES: usize = 8 * 1024;

    /// Provides the dynamic `/ready` and `/metrics` bodies for an ops listener.
    ///
    /// `ready` returns `(is_ready, detail)`; `metrics` returns the rendered
    /// Prometheus body. Both are `async` so a readiness probe can do real I/O
    /// (e.g. a dependency TCP connect) and a metrics snapshot can query a store.
    pub trait OpsProvider: Send + Sync + 'static {
        /// Readiness: `(ready, human-readable detail body)`.
        fn ready(&self) -> impl Future<Output = (bool, String)> + Send;
        /// Prometheus text-exposition metrics body.
        fn metrics(&self) -> impl Future<Output = String> + Send;
    }

    /// Serve `/health`, `/ready`, and `/metrics` on `listener` until `cancel`
    /// fires.
    ///
    /// One short-lived connection per request (`Connection: close`). This is an
    /// internal ops surface (liveness/readiness/metrics scrapes), so it favours
    /// simplicity over keep-alive throughput.
    ///
    /// # Errors
    ///
    /// Returns an `io::Error` only if `accept` fails; per-connection I/O errors
    /// are swallowed so one bad client never takes the listener down.
    pub async fn serve_ops<P: OpsProvider + Clone>(
        listener: TcpListener,
        provider: P,
        cancel: CancellationToken,
    ) -> std::io::Result<()> {
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                accept = listener.accept() => {
                    let (stream, _peer) = accept?;
                    let provider = provider.clone();
                    tokio::spawn(handle_ops_conn(stream, provider));
                }
            }
        }
        Ok(())
    }

    async fn handle_ops_conn<P: OpsProvider>(mut stream: TcpStream, provider: P) {
        let Some((method, target)) = read_request_line(&mut stream).await else {
            return;
        };
        let response = match classify_route(&method, &target) {
            OpsRoute::Health => OpsResponse::health_ok(),
            OpsRoute::Ready => {
                let (ready, detail) = provider.ready().await;
                OpsResponse::ready(ready, detail)
            }
            OpsRoute::Metrics => OpsResponse::metrics(provider.metrics().await),
            OpsRoute::NotFound => OpsResponse::not_found(),
        };
        let _ = stream.write_all(response.to_http_string().as_bytes()).await;
        let _ = stream.flush().await;
    }

    /// Read up to the end of the request headers and return `(method, target)`
    /// from the request line. Returns `None` on EOF, error, oversized headers,
    /// or a malformed request line.
    async fn read_request_line(stream: &mut TcpStream) -> Option<(String, String)> {
        let mut buf = Vec::with_capacity(512);
        let mut scratch = [0u8; 512];
        loop {
            if buf.len() > MAX_OPS_HEADER_BYTES {
                return None;
            }
            let n = stream.read(&mut scratch).await.ok()?;
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
}

#[cfg(not(loom))]
pub use listener::{OpsProvider, serve_ops};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_counter_and_gauge_families() {
        let metrics = vec![
            Metric::counter("tdw_demo_total", "demo total", 3.0),
            Metric::gauge("tdw_demo_inflight", "demo in-flight", 2.0),
        ];
        let body = render_prometheus(&metrics);
        assert_eq!(
            body,
            "# HELP tdw_demo_total demo total\n\
             # TYPE tdw_demo_total counter\n\
             tdw_demo_total 3\n\
             # HELP tdw_demo_inflight demo in-flight\n\
             # TYPE tdw_demo_inflight gauge\n\
             tdw_demo_inflight 2\n"
        );
    }

    #[test]
    fn shares_one_header_across_a_labelled_family() {
        let metrics = vec![
            Metric::counter("tdw_req", "requests by method", 5.0)
                .with_label("method", "tools/list"),
            Metric::counter("tdw_req", "requests by method", 7.0)
                .with_label("method", "tools/call"),
        ];
        let body = render_prometheus(&metrics);
        assert_eq!(
            body,
            "# HELP tdw_req requests by method\n\
             # TYPE tdw_req counter\n\
             tdw_req{method=\"tools/list\"} 5\n\
             tdw_req{method=\"tools/call\"} 7\n"
        );
    }

    #[test]
    fn escapes_label_values() {
        let metrics = vec![Metric::gauge("g", "h", 1.0).with_label("k", "a\"b\\c\nd")];
        let body = render_prometheus(&metrics);
        assert!(body.contains("k=\"a\\\"b\\\\c\\nd\""), "got: {body}");
    }

    #[test]
    fn formats_fractional_values_with_decimal() {
        let metrics = vec![Metric::gauge("ratio", "h", 0.5)];
        assert!(render_prometheus(&metrics).contains("ratio 0.5\n"));
    }

    #[test]
    fn classifies_routes_and_ignores_query_string() {
        assert_eq!(classify_route("GET", "/health"), OpsRoute::Health);
        assert_eq!(classify_route("GET", "/healthz"), OpsRoute::Health);
        assert_eq!(classify_route("GET", "/ready?x=1"), OpsRoute::Ready);
        assert_eq!(classify_route("GET", "/metrics"), OpsRoute::Metrics);
        assert_eq!(classify_route("GET", "/other"), OpsRoute::NotFound);
        assert_eq!(classify_route("POST", "/health"), OpsRoute::NotFound);
    }

    #[test]
    fn health_is_always_200() {
        let response = OpsResponse::health_ok();
        assert_eq!(response.status, 200);
        assert!(response.to_http_string().starts_with("HTTP/1.1 200 OK\r\n"));
    }

    #[test]
    fn ready_maps_to_200_or_503() {
        assert_eq!(OpsResponse::ready(true, "ok").status, 200);
        assert_eq!(OpsResponse::ready(false, "down").status, 503);
    }

    #[test]
    fn http_string_sets_content_length() {
        let response = OpsResponse::metrics("abc\n".to_string());
        let http = response.to_http_string();
        assert!(http.contains("Content-Length: 4\r\n"), "got: {http}");
        assert!(http.contains("version=0.0.4"), "got: {http}");
    }

    #[test]
    fn daemon_metrics_count_outcomes_and_in_flight() {
        use crate::EventMsg;
        use tdw_protocol::OpId;

        let metrics = DaemonMetrics::new();
        // One completed, one failed.
        metrics.dispatch_started();
        metrics.dispatch_finished(&[EventMsg::Completed {
            op_id: OpId::generated(),
            summary: None,
            result: None,
        }]);
        metrics.dispatch_started();
        metrics.dispatch_finished(&[EventMsg::Failed {
            op_id: OpId::generated(),
            error: "boom".to_string(),
        }]);
        // One still in flight (started, not finished).
        metrics.dispatch_started();

        let body = metrics.render();
        assert!(
            body.contains("tdw_daemon_dispatch_total{outcome=\"completed\"} 1"),
            "got: {body}"
        );
        assert!(
            body.contains("tdw_daemon_dispatch_total{outcome=\"failed\"} 1"),
            "got: {body}"
        );
        assert!(
            body.contains("tdw_daemon_dispatch_total{outcome=\"cancelled\"} 0"),
            "got: {body}"
        );
        assert!(
            body.contains("tdw_daemon_dispatch_in_flight 1"),
            "got: {body}"
        );
    }
}
