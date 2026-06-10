#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use std::net::SocketAddr;
use tdw_protocol::{EventMsg, OpEnvelope};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// P4: Feature-gated transport modules
// ---------------------------------------------------------------------------

// The transport modules use `tokio::net`, which tokio gates out under
// `--cfg loom`. They are excluded from the loom build (test-only config; the
// production binary never sets `--cfg loom`). See tests/loom_relay.rs.
#[cfg(all(feature = "transport-tcp", not(loom)))]
mod transport_tcp;
#[cfg(all(feature = "transport-tcp", not(loom)))]
pub use transport_tcp::serve_tcp;

#[cfg(all(unix, feature = "transport-uds", not(loom)))]
mod transport_uds;
#[cfg(all(unix, feature = "transport-uds", not(loom)))]
pub use transport_uds::serve_uds;

#[cfg(all(feature = "transport-http", not(loom)))]
mod transport_http;
#[cfg(all(feature = "functions-route", not(loom)))]
pub use transport_http::serve_functions_http;
#[cfg(all(feature = "transport-http", not(loom)))]
pub use transport_http::serve_http;

#[cfg(all(feature = "functions-route", not(loom)))]
pub mod functions_route;
#[cfg(all(feature = "functions-route", not(loom)))]
pub use functions_route::{FunctionEntry, FunctionsHandler, HandlerError, SigningConfig};

#[cfg(all(feature = "rest-api-route", not(loom)))]
pub mod rest_route;
#[cfg(all(feature = "rest-api-route", not(loom)))]
pub use rest_route::{RestApiHandler, RestError, serve_rest_http};

#[cfg(all(feature = "workspace-route", not(loom)))]
pub mod workspace_route;
#[cfg(all(feature = "workspace-route", not(loom)))]
pub use workspace_route::{WorkspaceConfig, serve_workspace_http};

#[cfg(all(feature = "agent-route", not(loom)))]
pub mod agent_route;
#[cfg(all(feature = "agent-route", not(loom)))]
pub use agent_route::{AgentBridgeHandler, serve_agent_http};

// Shared service-operability surface (/health, /ready, /metrics). The pure
// renderer and route classifier are always available; the async listener is
// gated out under `--cfg loom` (it uses `tokio::net`).
pub mod ops;

pub use tdw_config::DaemonTransport;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonEndpoint {
    pub transport: DaemonTransport,
    pub address: String,
}

pub type EndpointResult<T> = std::result::Result<T, EndpointError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndpointError {
    EmptyAddress,
    InvalidUdsAddress,
    InvalidTcpAddress,
    InvalidHttpSseAddress,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAddress => write!(formatter, "daemon endpoint address must not be empty"),
            Self::InvalidUdsAddress => write!(formatter, "invalid UDS daemon endpoint address"),
            Self::InvalidTcpAddress => write!(formatter, "invalid TCP daemon endpoint address"),
            Self::InvalidHttpSseAddress => {
                write!(formatter, "invalid HTTP/SSE daemon endpoint address")
            }
        }
    }
}

impl Error for EndpointError {}

impl DaemonEndpoint {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn validate(&self) -> EndpointResult<()> {
        validate_endpoint(self)
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_endpoint(endpoint: &DaemonEndpoint) -> EndpointResult<()> {
    let address = endpoint.address.trim();
    if address.is_empty() {
        return Err(EndpointError::EmptyAddress);
    }
    if address.chars().any(char::is_control) {
        return Err(match endpoint.transport {
            DaemonTransport::Tcp => EndpointError::InvalidTcpAddress,
            DaemonTransport::Uds => EndpointError::InvalidUdsAddress,
            DaemonTransport::HttpSse => EndpointError::InvalidHttpSseAddress,
        });
    }

    match endpoint.transport {
        DaemonTransport::Tcp => validate_tcp_address(address),
        DaemonTransport::Uds => validate_uds_address(address),
        DaemonTransport::HttpSse => validate_http_sse_address(address),
    }
}

fn validate_tcp_address(address: &str) -> EndpointResult<()> {
    if address.parse::<SocketAddr>().is_ok() {
        return Ok(());
    }

    let Some((host, port)) = address.rsplit_once(':') else {
        return Err(EndpointError::InvalidTcpAddress);
    };
    if host.is_empty()
        || host.contains(':')
        || host.contains('/')
        || host.contains('\\')
        || host.chars().any(char::is_whitespace)
        || port.parse::<u16>().is_err()
    {
        return Err(EndpointError::InvalidTcpAddress);
    }

    Ok(())
}

fn validate_uds_address(address: &str) -> EndpointResult<()> {
    if address.contains("://")
        || address.contains(';')
        || address.contains('|')
        || address.contains('&')
        || contains_parent_segment(address)
    {
        Err(EndpointError::InvalidUdsAddress)
    } else {
        Ok(())
    }
}

fn validate_http_sse_address(address: &str) -> EndpointResult<()> {
    if address.contains('\\') || address.chars().any(char::is_whitespace) {
        return Err(EndpointError::InvalidHttpSseAddress);
    }

    let host_and_path = address
        .strip_prefix("http://")
        .or_else(|| address.strip_prefix("https://"))
        .ok_or(EndpointError::InvalidHttpSseAddress)?;
    let host = host_and_path
        .split('/')
        .next()
        .ok_or(EndpointError::InvalidHttpSseAddress)?;

    if host.is_empty() || host.contains(';') || host.contains('|') || host.contains('&') {
        Err(EndpointError::InvalidHttpSseAddress)
    } else {
        Ok(())
    }
}

fn contains_parent_segment(value: &str) -> bool {
    value.split(['/', '\\']).any(|segment| segment == "..")
}

#[derive(Clone)]
pub struct SubmissionHandle {
    sender: mpsc::UnboundedSender<OpEnvelope>,
}

#[derive(Debug)]
pub struct SubmissionError {
    envelope: Box<OpEnvelope>,
}

impl SubmissionError {
    #[must_use]
    pub fn into_envelope(self) -> OpEnvelope {
        *self.envelope
    }
}

impl SubmissionHandle {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn submit(&self, envelope: OpEnvelope) -> std::result::Result<(), SubmissionError> {
        self.sender.send(envelope).map_err(|error| SubmissionError {
            envelope: Box::new(error.0),
        })
    }
}

pub struct AgentLoop {
    submissions: mpsc::UnboundedReceiver<OpEnvelope>,
    events: mpsc::UnboundedSender<EventMsg>,
}

impl AgentLoop {
    pub async fn run_once(&mut self) -> Option<EventMsg> {
        tokio::select! {
            Some(envelope) = self.submissions.recv() => {
                let event = EventMsg::Started { op_id: envelope.op_id };
                let _ = self.events.send(event.clone());
                Some(event)
            }
            else => None,
        }
    }
}

/// Async operation dispatcher used by the daemon's service loop.
///
/// Added in P1 of the integration cycle. The blanket `Send + Sync` bound plus
/// `async-trait` keeps the returned future `Send` so the service loop can
/// `tokio::spawn` it onto a multi-thread runtime. The existing `AgentLoop` and
/// `channel()` remain untouched for back-compat with the current
/// `client_event_sample` consumer and the test below.
#[async_trait::async_trait]
pub trait Dispatcher: Send + Sync {
    /// Dispatch a single `OpEnvelope`. Implementations should emit at least one
    /// `Started` followed by a terminal `Completed`/`Failed`/`Cancelled`
    /// per envelope, in order.
    async fn dispatch(&self, env: OpEnvelope) -> Vec<EventMsg>;
}

#[must_use]
pub fn channel() -> (
    SubmissionHandle,
    mpsc::UnboundedReceiver<EventMsg>,
    AgentLoop,
) {
    let (submission_tx, submission_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    (
        SubmissionHandle {
            sender: submission_tx,
        },
        event_rx,
        AgentLoop {
            submissions: submission_rx,
            events: event_tx,
        },
    )
}

// ---------------------------------------------------------------------------
// P2: EventSink + ServiceLoop
// ---------------------------------------------------------------------------

/// Error type returned by `EventSink` operations.
#[derive(Debug)]
pub struct SinkError(pub String);

impl std::fmt::Display for SinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sink error: {}", self.0)
    }
}

impl std::error::Error for SinkError {}

pub type SinkResult<T> = std::result::Result<T, SinkError>;

/// Durable persistence sink invoked by `ServiceLoop` for each dispatched event
/// and for the per-operation cost ledger record.
#[async_trait::async_trait]
pub trait EventSink: Send + Sync {
    /// Persist a single event from the dispatch result of `env`.
    async fn persist_event(
        &self,
        env: &OpEnvelope,
        event: &EventMsg,
        sequence: u64,
    ) -> SinkResult<()>;

    /// Record cost metadata for the completed operation `env`.
    async fn record_cost(&self, env: &OpEnvelope, backend: &str) -> SinkResult<()>;
}

/// Durable service loop that pairs a `Dispatcher` with an `EventSink`.
///
/// Added alongside the existing `AgentLoop` / `channel()` in P2. The existing
/// types are preserved for back-compat.
pub struct ServiceLoop<D: Dispatcher + 'static, S: EventSink + 'static> {
    submissions: mpsc::UnboundedReceiver<OpEnvelope>,
    events: mpsc::UnboundedSender<EventMsg>,
    dispatcher: D,
    sink: S,
    next_sequence: std::sync::atomic::AtomicU64,
    persist_failures: std::sync::atomic::AtomicU64,
    cost_failures: std::sync::atomic::AtomicU64,
    /// Optional dispatch metrics (outcome counters + in-flight gauge). `None`
    /// keeps the loop's behavior byte-for-byte unchanged; the daemon attaches a
    /// handle via [`ServiceLoop::with_metrics`] so its `/metrics` surface sees
    /// live dispatch counts.
    metrics: Option<ops::DaemonMetrics>,
}

/// Snapshot of durability-sink failure counts surfaced by [`ServiceLoop`] for
/// health/metrics. Non-zero values mean events or cost records failed to
/// persist while the op still completed (persistence here is best-effort), so a
/// health check / metrics scrape can detect a silent durability/audit/cost gap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServiceLoopFailures {
    /// Count of `persist_event` failures since the loop started.
    pub persist_event: u64,
    /// Count of `record_cost` failures since the loop started.
    pub record_cost: u64,
}

impl<D: Dispatcher + 'static, S: EventSink + 'static> ServiceLoop<D, S> {
    pub const fn new(
        submissions: mpsc::UnboundedReceiver<OpEnvelope>,
        events: mpsc::UnboundedSender<EventMsg>,
        dispatcher: D,
        sink: S,
    ) -> Self {
        Self {
            submissions,
            events,
            dispatcher,
            sink,
            next_sequence: std::sync::atomic::AtomicU64::new(1),
            persist_failures: std::sync::atomic::AtomicU64::new(0),
            cost_failures: std::sync::atomic::AtomicU64::new(0),
            metrics: None,
        }
    }

    /// Current durability-sink failure counts (see [`ServiceLoopFailures`]).
    /// Intended for a health endpoint / metrics scrape so a sink outage is
    /// observable rather than hidden.
    #[must_use]
    pub fn failure_counts(&self) -> ServiceLoopFailures {
        use std::sync::atomic::Ordering;
        ServiceLoopFailures {
            persist_event: self.persist_failures.load(Ordering::Relaxed),
            record_cost: self.cost_failures.load(Ordering::Relaxed),
        }
    }

    /// Attach a [`ops::DaemonMetrics`] handle so each dispatch updates the
    /// daemon's outcome counters and in-flight gauge. Consumes and returns
    /// `self` for builder use.
    #[must_use]
    pub fn with_metrics(mut self, metrics: ops::DaemonMetrics) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Receive one envelope, dispatch it, persist each event, send events on
    /// the channel, then record cost. Returns the events emitted, or `None` if
    /// the submission channel is closed.
    pub async fn run_once(&mut self) -> Option<Vec<EventMsg>> {
        let env = self.submissions.recv().await?;
        if let Some(metrics) = &self.metrics {
            metrics.dispatch_started();
        }
        let emitted = self.dispatcher.dispatch(env.clone()).await;
        if let Some(metrics) = &self.metrics {
            metrics.dispatch_finished(&emitted);
        }
        for event in &emitted {
            let seq = self
                .next_sequence
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Err(e) = self.sink.persist_event(&env, event, seq).await {
                // Don't bury a durability failure: bump a metric and surface a
                // signal on the event stream so a health check / consumer can
                // observe the audit/replay gap. The op still completes
                // (persistence is best-effort here).
                self.persist_failures
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // The raw error stays SERVER-SIDE only: the event stream fans
                // out to external transport clients, so the client-facing
                // payload carries just the (non-sensitive) sequence — never the
                // raw sink error, which can leak DB URLs / paths / query text.
                eprintln!("[ServiceLoop] persist_event failed (seq={seq}): {e}");
                let _ = self.events.send(EventMsg::DomainEvent {
                    op_id: env.op_id.clone(),
                    event_type: "service.persist_event_failed".to_string(),
                    payload: serde_json::json!({ "sequence": seq }),
                });
            }
            let _ = self.events.send(event.clone());
        }
        if let Err(e) = self.sink.record_cost(&env, "in-memory").await {
            self.cost_failures
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Raw error server-side only; client event carries no error detail.
            eprintln!("[ServiceLoop] record_cost failed: {e}");
            let _ = self.events.send(EventMsg::DomainEvent {
                op_id: env.op_id.clone(),
                event_type: "service.record_cost_failed".to_string(),
                payload: serde_json::Value::Null,
            });
        }
        Some(emitted)
    }
}

/// Factory that mirrors `channel()` but wires a `Dispatcher` and `EventSink`
/// into a `ServiceLoop`.
pub fn service_channel<D: Dispatcher + 'static, S: EventSink + 'static>(
    dispatcher: D,
    sink: S,
) -> (
    SubmissionHandle,
    mpsc::UnboundedReceiver<EventMsg>,
    ServiceLoop<D, S>,
) {
    let (submission_tx, submission_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    (
        SubmissionHandle {
            sender: submission_tx,
        },
        event_rx,
        ServiceLoop::new(submission_rx, event_tx, dispatcher, sink),
    )
}

// ---------------------------------------------------------------------------
// P3: CancellationToken re-export, outbox→bus relay, serve lifecycle
// ---------------------------------------------------------------------------

pub use tokio_util::sync::CancellationToken;

/// Returns a human-readable label for the transport variant (useful for logging).
#[must_use]
pub const fn transport_label(t: DaemonTransport) -> &'static str {
    match t {
        DaemonTransport::Tcp => "tcp",
        DaemonTransport::Uds => "uds",
        DaemonTransport::HttpSse => "http-sse",
    }
}

/// Spawn the in-memory outbox→bus relay.
///
/// The task polls the outbox for
/// pending records every `tick`, publishes each on the bus, and marks it
/// dispatched. Cooperative shutdown via the supplied `CancellationToken`.
///
/// Mutex poisoning is recovered (the guard is taken via `into_inner`) rather
/// than panicked on: a panic in some unrelated lock-holder must not silently
/// kill event delivery. The outbox/bus are simple in-memory collections, so a
/// recovered guard is safe to keep using. `serve` additionally treats this
/// task ending before cancellation as an error (see [`serve`]).
pub fn spawn_inmemory_relay(
    outbox: std::sync::Arc<std::sync::Mutex<tdw_outbox::InMemoryOutbox>>,
    bus: std::sync::Arc<std::sync::Mutex<tdw_bus::EventBus>>,
    tick: std::time::Duration,
    cancel: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_seq: u64 = 0;
        loop {
            // Drain all pending records in one tick.
            let pending = {
                let guard = outbox
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.pending_after(last_seq)
            };
            for record in &pending {
                // Publish a fresh EventEnvelope of the same payload onto the bus.
                let envelope = record.envelope.clone();
                {
                    let mut bus_guard = bus
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    bus_guard.publish(envelope);
                }
                {
                    let mut outbox_guard = outbox
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    outbox_guard.mark_dispatched(record.sequence);
                }
                if record.sequence > last_seq {
                    last_seq = record.sequence;
                }
            }
            // Wait for next tick or cancellation.
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }
        }
    })
}

/// Run the daemon's service loop + relay until cancelled.
///
/// Concretely:
/// * Drives a `ServiceLoop<D, S>` until its `submissions` channel closes or the
///   `CancellationToken` fires.
/// * Co-spawns the in-memory outbox→bus relay (handed off via `relay`).
/// * Listens for `tokio::signal::ctrl_c()` and cancels the token.
/// * Listens for the loop emitting `EventMsg::Completed` after dispatching
///   an `Op::Shutdown` and triggers cancellation.
///
/// # Errors
///
/// Returns an error variant if the underlying operation fails.
/// Resolve when the process receives a graceful-stop signal: `ctrl_c` on every
/// platform, plus `SIGTERM` on unix (what `docker stop`/systemd/Kubernetes send
/// at the end of the termination grace period). Long-running serve loops
/// (`tdw-mcp`, the worker, the daemon) await this to begin a bounded drain.
///
/// Excluded from the loom build (it uses `tokio::signal`).
#[cfg(not(loom))]
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        // If SIGTERM cannot be installed, fall back to ctrl-c only.
        let Ok(mut sigterm) = signal(SignalKind::terminate()) else {
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Drive the service loop until shutdown, then drain the relay task.
///
/// # Errors
///
/// Returns [`SinkError`] if the underlying service loop or event sink fails
/// while dispatching or draining events.
pub async fn serve<D: Dispatcher + 'static, S: EventSink + 'static>(
    mut service_loop: ServiceLoop<D, S>,
    mut relay: tokio::task::JoinHandle<()>,
    shutdown: tokio_util::sync::CancellationToken,
) -> std::result::Result<(), SinkError> {
    // `tokio::signal` is gated out by tokio under `--cfg loom`; the ctrl_c
    // watcher is only wired in the normal (non-loom) build. The loom model
    // exercises the relay locking logic, not signal handling.
    #[cfg(not(loom))]
    let shutdown_for_signal = shutdown.clone();
    #[cfg(not(loom))]
    let signal_task: tokio::task::JoinHandle<()> = tokio::spawn(async move {
        // Cancel on ctrl-c (all platforms) or SIGTERM (unix). SIGTERM is what
        // container/systemd stop sends, so a graceful drain hinges on it.
        shutdown_signal().await;
        shutdown_for_signal.cancel();
    });

    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            // The relay only ends on cancellation. If it resolves while we are
            // still serving, it died unexpectedly (e.g. a panic) — surface it
            // and shut down rather than discarding the JoinError and silently
            // serving with no event delivery.
            relay_result = &mut relay => {
                shutdown.cancel();
                #[cfg(not(loom))]
                signal_task.abort();
                return Err(SinkError(match relay_result {
                    Ok(()) => "outbox→bus relay terminated unexpectedly".to_string(),
                    Err(join_error) => format!("outbox→bus relay task failed: {join_error}"),
                }));
            }
            maybe = service_loop.run_once() => {
                match maybe {
                    None => break, // submissions channel closed
                    Some(events) => {
                        // If any event indicates a Shutdown dispatched, trigger cancellation.
                        if events.iter().any(|e| matches!(e, EventMsg::Completed { result: Some(value), .. } if value.get("shutdown").and_then(|v| v.as_str()) == Some("requested"))) {
                            shutdown.cancel();
                            break;
                        }
                    }
                }
            }
        }
    }

    // Drain: ensure the relay observes cancellation, then await its (now
    // expected) termination.
    shutdown.cancel();
    let _ = relay.await;
    #[cfg(not(loom))]
    signal_task.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, Op, SessionId};

    #[tokio::test]
    async fn agent_loop_emits_started_event_from_submission_queue() {
        let (handle, mut events, mut loop_runner) = channel();
        let envelope = OpEnvelope::new(
            SessionId::new("session-1").expect("session id"),
            1,
            ActorRef {
                actor_id: "user".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::Shutdown,
        );

        handle.submit(envelope).expect("submission accepted");
        let event = loop_runner.run_once().await.expect("event emitted");

        assert!(matches!(event, EventMsg::Started { .. }));
        assert!(matches!(
            events.recv().await,
            Some(EventMsg::Started { .. })
        ));
    }

    // -----------------------------------------------------------------------
    // P3 tests
    // -----------------------------------------------------------------------

    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tdw_bus::EventBus;
    use tdw_event::sample_event;
    use tdw_outbox::InMemoryOutbox;

    struct FakeDispatcher;

    #[async_trait::async_trait]
    impl Dispatcher for FakeDispatcher {
        async fn dispatch(&self, env: OpEnvelope) -> Vec<EventMsg> {
            vec![EventMsg::Started { op_id: env.op_id }]
        }
    }

    struct ShutdownDispatcher;

    #[async_trait::async_trait]
    impl Dispatcher for ShutdownDispatcher {
        async fn dispatch(&self, env: OpEnvelope) -> Vec<EventMsg> {
            match &env.op {
                Op::Shutdown => vec![
                    EventMsg::Started {
                        op_id: env.op_id.clone(),
                    },
                    EventMsg::Completed {
                        op_id: env.op_id,
                        summary: None,
                        result: Some(json!({"shutdown": "requested"})),
                    },
                ],
                _ => vec![EventMsg::Started { op_id: env.op_id }],
            }
        }
    }

    struct FakeSink;

    #[async_trait::async_trait]
    impl EventSink for FakeSink {
        async fn persist_event(
            &self,
            _env: &OpEnvelope,
            _event: &EventMsg,
            _sequence: u64,
        ) -> SinkResult<()> {
            Ok(())
        }

        async fn record_cost(&self, _env: &OpEnvelope, _backend: &str) -> SinkResult<()> {
            Ok(())
        }
    }

    fn make_envelope(op: Op) -> OpEnvelope {
        OpEnvelope::new(
            SessionId::new("session-p3").expect("session id"),
            1,
            ActorRef {
                actor_id: "test".to_string(),
                kind: ActorKind::System,
                tenant_id: None,
            },
            op,
        )
    }

    struct FailingSink;

    #[async_trait::async_trait]
    impl EventSink for FailingSink {
        async fn persist_event(
            &self,
            _env: &OpEnvelope,
            _event: &EventMsg,
            _sequence: u64,
        ) -> SinkResult<()> {
            Err(SinkError("persist boom".to_string()))
        }

        async fn record_cost(&self, _env: &OpEnvelope, _backend: &str) -> SinkResult<()> {
            Err(SinkError("cost boom".to_string()))
        }
    }

    #[tokio::test]
    async fn run_once_surfaces_sink_failures_via_counts_and_events() {
        let (handle, mut events, mut loop_runner) = service_channel(FakeDispatcher, FailingSink);
        handle
            .submit(make_envelope(Op::Shutdown))
            .expect("submit accepted");

        let emitted = loop_runner
            .run_once()
            .await
            .expect("run_once yields events");
        assert_eq!(emitted.len(), 1, "FakeDispatcher emits one event");

        // Both durability failures are now counted (not silently swallowed).
        let counts = loop_runner.failure_counts();
        assert_eq!(counts.persist_event, 1);
        assert_eq!(counts.record_cost, 1);

        // ...and surfaced on the event stream so a consumer/health check sees
        // them. Order: persist-failure signal, the dispatched event, then the
        // cost-failure signal.
        let first = events.recv().await.expect("persist failure event");
        assert!(matches!(
            &first,
            EventMsg::DomainEvent { event_type, .. } if event_type == "service.persist_event_failed"
        ));
        let second = events.recv().await.expect("dispatched event");
        assert!(matches!(second, EventMsg::Started { .. }));
        let third = events.recv().await.expect("cost failure event");
        assert!(matches!(
            &third,
            EventMsg::DomainEvent { event_type, .. } if event_type == "service.record_cost_failed"
        ));
    }

    #[tokio::test]
    async fn relay_drains_outbox_into_bus_and_marks_dispatched() {
        let outbox = Arc::new(Mutex::new(InMemoryOutbox::default()));
        let bus = Arc::new(Mutex::new(EventBus::new(64)));

        {
            let mut o = outbox.lock().expect("lock");
            o.append(sample_event("test-a"));
            o.append(sample_event("test-b"));
        }

        let cancel = CancellationToken::new();
        let handle = spawn_inmemory_relay(
            outbox.clone(),
            bus.clone(),
            Duration::from_millis(5),
            cancel.clone(),
        );

        // Wait up to 500 ms for both sequences to be dispatched.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        loop {
            let pending = outbox.lock().expect("lock").pending_after(0);
            if pending.is_empty() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        cancel.cancel();
        let _ = handle.await;

        let entries = bus.lock().expect("lock").read_from(1);
        assert_eq!(entries.len(), 2, "bus should have both events");

        let pending = outbox.lock().expect("lock").pending_after(0);
        assert!(
            pending.is_empty(),
            "all outbox records should be dispatched"
        );
    }

    #[tokio::test]
    async fn serve_returns_when_cancellation_fires() {
        let (handle, _events, service_loop) = service_channel(FakeDispatcher, FakeSink);
        drop(handle); // close submission channel so serve won't hang on run_once

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        // Dummy relay that just waits for cancellation.
        let relay = tokio::spawn(async move {
            cancel_clone.cancelled().await;
        });

        let cancel_for_serve = cancel.clone();
        let serve_join =
            tokio::spawn(async move { serve(service_loop, relay, cancel_for_serve).await });

        // Give serve a moment then cancel.
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();

        let result = tokio::time::timeout(Duration::from_secs(1), serve_join).await;
        assert!(result.is_ok(), "serve should complete within timeout");
        assert!(
            result.expect("join ok").expect("no panic").is_ok(),
            "serve returns Ok"
        );
    }

    #[tokio::test]
    async fn serve_terminates_on_dispatched_shutdown() {
        let (submission, _events, service_loop) = service_channel(ShutdownDispatcher, FakeSink);

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let relay = tokio::spawn(async move {
            cancel_clone.cancelled().await;
        });

        let cancel_for_serve = cancel.clone();
        let serve_join =
            tokio::spawn(async move { serve(service_loop, relay, cancel_for_serve).await });

        // Submit a Shutdown op.
        submission
            .submit(make_envelope(Op::Shutdown))
            .expect("submit shutdown");

        let result = tokio::time::timeout(Duration::from_secs(2), serve_join).await;
        assert!(result.is_ok(), "serve should complete within timeout");
        assert!(
            result.expect("join ok").expect("no panic").is_ok(),
            "serve returns Ok"
        );
        assert!(
            cancel.is_cancelled(),
            "token should be cancelled after shutdown"
        );
    }

    #[tokio::test]
    async fn serve_errors_when_relay_dies_unexpectedly() {
        // Keep the submission channel open (handle alive) so run_once awaits;
        // the relay ends immediately, simulating a dead relay. serve must
        // surface that as an error rather than hang or silently return Ok.
        let (_handle, _events, service_loop) = service_channel(FakeDispatcher, FakeSink);
        let cancel = CancellationToken::new();
        let relay = tokio::spawn(async {}); // ends immediately

        let result =
            tokio::time::timeout(Duration::from_secs(1), serve(service_loop, relay, cancel)).await;
        let outcome = result.expect("serve must not hang");
        assert!(
            matches!(&outcome, Err(SinkError(msg)) if msg.contains("relay terminated unexpectedly")),
            "expected relay-death error, got: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn relay_recovers_from_poisoned_outbox_mutex() {
        let outbox = Arc::new(Mutex::new(InMemoryOutbox::default()));
        let bus = Arc::new(Mutex::new(EventBus::new(64)));
        {
            let mut o = outbox.lock().expect("lock");
            o.append(sample_event("poison-test"));
        }

        // Poison the outbox mutex by panicking while holding it.
        let poison_target = outbox.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_target.lock().expect("lock");
            panic!("intentionally poison the outbox mutex");
        })
        .join();
        assert!(outbox.lock().is_err(), "outbox mutex should be poisoned");

        // The relay must keep draining despite the poison (recover the guard,
        // don't panic) — otherwise a stray panic anywhere silently kills event
        // delivery.
        let cancel = CancellationToken::new();
        let handle = spawn_inmemory_relay(
            outbox.clone(),
            bus.clone(),
            Duration::from_millis(5),
            cancel.clone(),
        );

        let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
        loop {
            let pending = outbox
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pending_after(0);
            if pending.is_empty() || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        cancel.cancel();
        let _ = handle.await;

        let pending = outbox
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pending_after(0);
        assert!(
            pending.is_empty(),
            "relay should drain even a poisoned-mutex outbox"
        );
    }

    // -----------------------------------------------------------------------
    // P4 transport tests
    // -----------------------------------------------------------------------

    #[cfg(all(feature = "transport-tcp", not(loom)))]
    mod tcp_transport_tests {
        use super::*;
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        async fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> std::io::Result<()> {
            let len = u32::try_from(bytes.len())
                .expect("test frame length fits u32")
                .to_be_bytes();
            stream.write_all(&len).await?;
            stream.write_all(bytes).await?;
            stream.flush().await
        }

        async fn read_frame_client(stream: &mut TcpStream) -> std::io::Result<Option<Vec<u8>>> {
            let mut len_buf = [0u8; 4];
            match stream.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(e) => return Err(e),
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            Ok(Some(buf))
        }

        fn make_test_envelope() -> OpEnvelope {
            OpEnvelope::new(
                tdw_protocol::SessionId::new("test-session").expect("session id"),
                1,
                tdw_protocol::ActorRef {
                    actor_id: "test".to_string(),
                    kind: tdw_protocol::ActorKind::System,
                    tenant_id: None,
                },
                tdw_protocol::Op::Shutdown,
            )
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn tcp_roundtrips_op_envelope_and_event_msg() {
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
                let addr = listener.local_addr().expect("local addr");

                let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<OpEnvelope>();
                let (evt_tx, evt_rx) = mpsc::unbounded_channel::<EventMsg>();
                let handle = SubmissionHandle { sender: sub_tx };

                let cancel = CancellationToken::new();
                let cancel_srv = cancel.clone();
                tokio::spawn(async move {
                    serve_tcp(listener, handle, evt_rx, cancel_srv)
                        .await
                        .expect("serve_tcp");
                });

                // Connect a client.
                let mut client = TcpStream::connect(addr).await.expect("connect");

                // Send an OpEnvelope frame.
                let env = make_test_envelope();
                let json = serde_json::to_vec(&env).expect("serialize envelope");
                write_frame(&mut client, &json).await.expect("write frame");

                // Wait for the envelope to arrive at the submission receiver.
                let received_env = sub_rx.recv().await.expect("envelope received");
                assert_eq!(received_env.op_id, env.op_id);

                // Emit an EventMsg via the events channel.
                let event = EventMsg::Started {
                    op_id: env.op_id.clone(),
                };
                evt_tx.send(event.clone()).expect("send event");

                // Read the framed EventMsg back from the server.
                let frame = read_frame_client(&mut client)
                    .await
                    .expect("read frame")
                    .expect("frame present");
                let decoded: EventMsg = serde_json::from_slice(&frame).expect("deserialize event");
                assert!(matches!(decoded, EventMsg::Started { .. }));

                cancel.cancel();
            })
            .await;

            result.expect("test timed out");
        }
    }

    #[cfg(feature = "transport-http")]
    mod http_transport_tests {
        use super::*;
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        fn make_test_envelope() -> OpEnvelope {
            OpEnvelope::new(
                tdw_protocol::SessionId::new("http-session").expect("session id"),
                1,
                tdw_protocol::ActorRef {
                    actor_id: "test".to_string(),
                    kind: tdw_protocol::ActorKind::System,
                    tenant_id: None,
                },
                tdw_protocol::Op::Shutdown,
            )
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn http_post_op_and_get_events_sse_streams_response() {
            let result = tokio::time::timeout(Duration::from_secs(3), async {
                let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
                let addr = listener.local_addr().expect("local addr");

                let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<OpEnvelope>();
                let (evt_tx, evt_rx) = mpsc::unbounded_channel::<EventMsg>();
                let handle = SubmissionHandle { sender: sub_tx };

                let cancel = CancellationToken::new();
                let cancel_srv = cancel.clone();
                tokio::spawn(async move {
                    serve_http(listener, handle, evt_rx, cancel_srv).await.expect("serve_http");
                });

                // Give the server a moment to be ready.
                tokio::time::sleep(Duration::from_millis(10)).await;

                let env = make_test_envelope();
                let body = serde_json::to_vec(&env).expect("serialize");

                // POST /op
                let mut post_conn = TcpStream::connect(addr).await.expect("connect post");
                let request = format!(
                    "POST /op HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                post_conn.write_all(request.as_bytes()).await.expect("write request");
                post_conn.write_all(&body).await.expect("write body");
                post_conn.flush().await.expect("flush");

                // Read 202 response.
                let mut resp_buf = vec![0u8; 512];
                let n = post_conn.read(&mut resp_buf).await.expect("read response");
                let resp_str = std::str::from_utf8(&resp_buf[..n]).expect("utf8");
                assert!(resp_str.starts_with("HTTP/1.1 202"), "expected 202, got: {resp_str}");

                // Confirm envelope arrived.
                let received = sub_rx.recv().await.expect("envelope received");
                assert_eq!(received.op_id, env.op_id);

                // GET /events — open SSE stream.
                let mut sse_conn = TcpStream::connect(addr).await.expect("connect sse");
                sse_conn.write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n").await.expect("write get");
                sse_conn.flush().await.expect("flush");

                // Read past the response headers.
                let mut sse_buf = Vec::new();
                loop {
                    let mut chunk = vec![0u8; 512];
                    let n = sse_conn.read(&mut chunk).await.expect("read sse");
                    sse_buf.extend_from_slice(&chunk[..n]);
                    let s = String::from_utf8_lossy(&sse_buf);
                    if s.contains("\r\n\r\n") {
                        break;
                    }
                }

                // Emit an event.
                let event = EventMsg::Started { op_id: env.op_id.clone() };
                evt_tx.send(event).expect("send event");

                // Read one SSE data line.
                let mut line_buf = Vec::new();
                loop {
                    let mut chunk = vec![0u8; 512];
                    let n = sse_conn.read(&mut chunk).await.expect("read event");
                    if n == 0 { break; }
                    line_buf.extend_from_slice(&chunk[..n]);
                    let s = String::from_utf8_lossy(&line_buf);
                    if s.contains("data:") {
                        break;
                    }
                }
                let sse_str = String::from_utf8_lossy(&line_buf);
                assert!(sse_str.contains("data:"), "expected SSE data line, got: {sse_str}");

                cancel.cancel();
            })
            .await;

            result.expect("test timed out");
        }

        // -------------------------------------------------------------------
        // fu-04: multi-client concurrent-subscriber broadcast fan-out tests
        //
        // These pin the broadcast semantics of `serve_http`: a 1024-slot
        // `broadcast::channel` fans every pumped `EventMsg` out to all live
        // `GET /events` subscribers, with no replay/history for late joiners,
        // and a lagging consumer is dropped (`RecvError::Lagged`) without
        // starving the others.
        //
        // Out of scope (deferred — not exercised here): SSE auth,
        // `Last-Event-ID` reconnect/replay, TLS.
        // -------------------------------------------------------------------

        /// Open an SSE subscriber: connect, send `GET /events` with the
        /// `Accept: text/event-stream` header, and consume the response up to
        /// and including the `\r\n\r\n` header terminator.
        ///
        /// Returns `(stream, seed)` where `seed` is any bytes that were read
        /// from the socket beyond the header terminator in the same TCP segment.
        /// On Linux (and under load) the SSE header block and the first event
        /// bytes can coalesce into one read; callers MUST pass `seed` as the
        /// first argument to `read_sse_events` so those bytes are not silently
        /// discarded.
        ///
        /// The subscription point is `tx.subscribe()` inside `serve_http`'s accept
        /// loop (`transport_http.rs:63`), which runs before the SSE handler task
        /// is spawned. Waiting for the HTTP header reply means the server has
        /// already subscribed this receiver to the broadcast — guaranteeing zero
        /// event loss for any send that happens after `open_sse` returns.
        ///
        /// The header-read is bounded by a 2-second timeout so a stalled
        /// handshake fails fast with a clear message.
        async fn open_sse(addr: std::net::SocketAddr) -> (TcpStream, Vec<u8>) {
            let mut stream = TcpStream::connect(addr).await.expect("connect sse");
            stream
                .write_all(
                    b"GET /events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n",
                )
                .await
                .expect("write GET /events");
            stream.flush().await.expect("flush sse request");

            let seed = tokio::time::timeout(Duration::from_secs(2), async {
                let mut buf = Vec::new();
                loop {
                    let mut chunk = [0u8; 512];
                    let n = stream.read(&mut chunk).await.expect("read sse headers");
                    assert_ne!(n, 0, "connection closed before SSE headers completed");
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        // Return any bytes that arrived after the header
                        // terminator in the same read — these are real event
                        // data and must not be dropped.
                        return buf[pos + 4..].to_vec();
                    }
                }
            })
            .await
            .expect("open_sse: timed out reading SSE response headers");

            (stream, seed)
        }

        /// POST an `OpEnvelope` to `/op` on a fresh connection and assert the
        /// server answers `202 Accepted`.
        async fn post_op(addr: std::net::SocketAddr, env: &OpEnvelope) {
            let body = serde_json::to_vec(env).expect("serialize envelope");
            let mut conn = TcpStream::connect(addr).await.expect("connect post");
            let request = format!(
                "POST /op HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            conn.write_all(request.as_bytes())
                .await
                .expect("write request line");
            conn.write_all(&body).await.expect("write body");
            conn.flush().await.expect("flush post");

            let mut resp = vec![0u8; 256];
            let n = conn.read(&mut resp).await.expect("read post response");
            let resp_str = std::str::from_utf8(&resp[..n]).expect("utf8 response");
            assert!(
                resp_str.starts_with("HTTP/1.1 202"),
                "expected 202 Accepted, got: {resp_str}"
            );
        }

        /// Read exactly `expected` SSE events (`data: ...\n\n` frames) from an
        /// already-handshaken stream, returning the decoded JSON payloads in
        /// arrival order. Takes ownership of the stream so it can be moved into
        /// a `tokio::spawn` closure for concurrent multi-subscriber draining.
        ///
        /// `seed` is the leftover bytes returned by `open_sse` (bytes that
        /// arrived in the same TCP read as the header terminator on Linux). They
        /// are parsed first, before any further socket reads, so no event data
        /// is lost.
        ///
        /// Bounded by `timeout` so a stall fails fast instead of hanging the
        /// test harness forever.
        async fn read_sse_events(
            mut stream: TcpStream,
            seed: Vec<u8>,
            expected: usize,
            timeout: Duration,
        ) -> Vec<String> {
            let collected = tokio::time::timeout(timeout, async {
                // Prepend any bytes that coalesced with the HTTP headers on the
                // same TCP segment (common on Linux under low latency).
                let mut acc = seed;
                let mut payloads = Vec::new();
                loop {
                    // Drain any complete frames already in the accumulator
                    // before blocking on the socket.
                    while let Some(pos) = acc.windows(2).position(|w| w == b"\n\n") {
                        let frame: Vec<u8> = acc.drain(..pos + 2).collect();
                        let text = String::from_utf8_lossy(&frame);
                        for line in text.lines() {
                            if let Some(json) = line.strip_prefix("data: ") {
                                payloads.push(json.to_string());
                            }
                        }
                        if payloads.len() >= expected {
                            return payloads;
                        }
                    }

                    let mut chunk = [0u8; 1024];
                    let n = stream.read(&mut chunk).await.expect("read sse event");
                    if n == 0 {
                        // Peer closed; return what we have so callers can assert
                        // on a short (lagging/disconnected) read.
                        break;
                    }
                    acc.extend_from_slice(&chunk[..n]);
                }
                payloads
            })
            .await;
            collected.unwrap_or_else(|_| panic!("timed out waiting for {expected} SSE events"))
        }

        /// Build an envelope with a fresh, unique `op_id` (`UUIDv7` via
        /// `OpEnvelope::new`) so emitted events can be told apart across the
        /// broadcast. `OpId` has no public string constructor, so callers that
        /// need to assert on identity capture `env.op_id.as_str()`.
        fn make_seq_envelope() -> OpEnvelope {
            make_test_envelope()
        }

        /// Emit a realistic `[Started, Completed]` event pair for `env` over the
        /// mpsc events channel; `serve_http` pumps these onto the broadcast.
        fn emit_started_completed(evt_tx: &mpsc::UnboundedSender<EventMsg>, env: &OpEnvelope) {
            evt_tx
                .send(EventMsg::Started {
                    op_id: env.op_id.clone(),
                })
                .expect("send Started");
            evt_tx
                .send(EventMsg::Completed {
                    op_id: env.op_id.clone(),
                    summary: None,
                    result: None,
                })
                .expect("send Completed");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn http_broadcasts_to_two_concurrent_subscribers() {
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
                let addr = listener.local_addr().expect("local addr");

                let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<OpEnvelope>();
                let (evt_tx, evt_rx) = mpsc::unbounded_channel::<EventMsg>();
                let handle = SubmissionHandle { sender: sub_tx };

                let cancel = CancellationToken::new();
                let cancel_srv = cancel.clone();
                let serve_join = tokio::spawn(async move {
                    serve_http(listener, handle, evt_rx, cancel_srv)
                        .await
                        .expect("serve_http");
                });

                // Two concurrent subscribers. open_sse completes the HTTP
                // header handshake, which means serve_http's accept loop has
                // called tx.subscribe() and spawned each SSE handler task —
                // both broadcast receivers are live before any POST.
                let (sub_a, seed_a) = open_sse(addr).await;
                let (sub_b, seed_b) = open_sse(addr).await;

                // POST two ops; for each, drain the submission so we know it
                // landed, then emit its [Started, Completed] pair.
                for _ in 0..2u64 {
                    let env = make_seq_envelope();
                    post_op(addr, &env).await;
                    let received = sub_rx.recv().await.expect("op received");
                    assert_eq!(received.op_id, env.op_id);
                    emit_started_completed(&evt_tx, &env);
                }

                // Drain both subscribers CONCURRENTLY. Reading them serially
                // would stall: while draining A, nobody reads B, so B's socket
                // send-buffer fills and the server task for B blocks, which can
                // in turn cause A's inner timeout to blow under CI load.
                let reader_a = tokio::spawn(async move {
                    read_sse_events(sub_a, seed_a, 4, Duration::from_secs(2)).await
                });
                let reader_b = tokio::spawn(async move {
                    read_sse_events(sub_b, seed_b, 4, Duration::from_secs(2)).await
                });
                let events_a = reader_a.await.expect("reader A join");
                let events_b = reader_b.await.expect("reader B join");
                assert_eq!(events_a.len(), 4, "subscriber A should see all 4 events");
                assert_eq!(events_b.len(), 4, "subscriber B should see all 4 events");

                // Deterministic teardown: cancel then await the server task.
                // The streams were moved into the reader tasks and are already
                // dropped when those tasks completed.
                cancel.cancel();
                serve_join.await.expect("serve_http join");
            })
            .await;

            result.expect("test timed out");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn http_late_subscriber_misses_pre_subscribe_events() {
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
                let addr = listener.local_addr().expect("local addr");

                let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<OpEnvelope>();
                let (evt_tx, evt_rx) = mpsc::unbounded_channel::<EventMsg>();
                let handle = SubmissionHandle { sender: sub_tx };

                let cancel = CancellationToken::new();
                let cancel_srv = cancel.clone();
                let serve_join = tokio::spawn(async move {
                    serve_http(listener, handle, evt_rx, cancel_srv)
                        .await
                        .expect("serve_http");
                });

                // Open a WARMUP subscriber first so we have a concrete receiver
                // to prove op #1's events have been fully broadcast before the
                // late subscriber's tx.subscribe() call.
                let (warmup, warmup_seed) = open_sse(addr).await;

                // POST op #1 and emit its [Started, Completed] pair.
                let env1 = make_seq_envelope();
                let op1 = env1.op_id.as_str().to_string();
                post_op(addr, &env1).await;
                let r1 = sub_rx.recv().await.expect("op1 received");
                assert_eq!(r1.op_id, env1.op_id);
                emit_started_completed(&evt_tx, &env1);

                // Reading op #1's 2 events from the warmup sub is the
                // deterministic barrier: it proves the pump has broadcast both
                // events before we proceed. No timing assumption required.
                let warmup_events =
                    read_sse_events(warmup, warmup_seed, 2, Duration::from_secs(2)).await;
                assert_eq!(warmup_events.len(), 2, "warmup sub should see op #1");

                // NOW subscribe the late joiner. Because op #1 is fully
                // broadcast, this receiver's tx.subscribe() cannot see it —
                // broadcast channels have no history for new receivers.
                let (sub, sub_seed) = open_sse(addr).await;

                // Op #2 happens after the late subscription.
                let env2 = make_seq_envelope();
                let op2 = env2.op_id.as_str().to_string();
                post_op(addr, &env2).await;
                let r2 = sub_rx.recv().await.expect("op2 received");
                assert_eq!(r2.op_id, env2.op_id);
                emit_started_completed(&evt_tx, &env2);

                // The late subscriber sees exactly op #2's two events, and they
                // reference op #2 (never op #1).
                let events = read_sse_events(sub, sub_seed, 2, Duration::from_secs(2)).await;
                assert_eq!(events.len(), 2, "late subscriber should see only op #2");
                for payload in &events {
                    assert!(
                        payload.contains(&op2),
                        "late subscriber must see op #2 event: {payload}"
                    );
                    assert!(
                        !payload.contains(&op1),
                        "late subscriber must not replay op #1: {payload}"
                    );
                }

                // Both streams were moved into read_sse_events; already dropped.
                cancel.cancel();
                serve_join.await.expect("serve_http join");
            })
            .await;

            result.expect("test timed out");
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn http_slow_subscriber_does_not_starve_others() {
            let result = tokio::time::timeout(Duration::from_secs(20), async {
                // 2000 ops × 2 events = 4000 events, well above the 1024-slot
                // broadcast buffer, so any platform that does lag-drop will hit it.
                const OPS: u64 = 2000;
                #[allow(clippy::cast_possible_truncation)] // OPS is the const 2000
                const EXPECTED_EVENTS: usize = (OPS as usize) * 2;

                let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
                let addr = listener.local_addr().expect("local addr");

                let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<OpEnvelope>();
                let (evt_tx, evt_rx) = mpsc::unbounded_channel::<EventMsg>();
                let handle = SubmissionHandle { sender: sub_tx };

                let cancel = CancellationToken::new();
                let cancel_srv = cancel.clone();
                let serve_join = tokio::spawn(async move {
                    serve_http(listener, handle, evt_rx, cancel_srv)
                        .await
                        .expect("serve_http");
                });

                // Fast reader drains eagerly; slow peer never reads its socket.
                let (fast, fast_seed) = open_sse(addr).await;
                // _slow is intentionally held open and never read throughout the
                // flood — it acts as an unresponsive peer. Whether the OS socket
                // buffer absorbs all events or the server hits RecvError::Lagged
                // is platform-dependent (Linux buffers are large enough to absorb
                // everything; other platforms drop early). We do NOT assert on
                // the slow sub's event count because that is non-deterministic.
                // The meaningful, portable guarantee is that the FAST subscriber
                // receives every event while the unresponsive peer is connected.
                let (_slow_stream, _slow_seed) = open_sse(addr).await;

                // Drain the fast reader concurrently so the server-side write to
                // the fast connection never blocks while we are POSTing/emitting.
                let fast_reader = tokio::spawn(async move {
                    read_sse_events(fast, fast_seed, EXPECTED_EVENTS, Duration::from_secs(15)).await
                });

                for _ in 0..OPS {
                    let env = make_seq_envelope();
                    handle_submit_and_emit(addr, &mut sub_rx, &evt_tx, &env).await;
                }

                // Core anti-starvation assertion: the fast subscriber receives
                // ALL events while an unresponsive peer is connected throughout.
                let fast_events = fast_reader.await.expect("fast reader join");
                assert_eq!(
                    fast_events.len(),
                    EXPECTED_EVENTS,
                    "fast subscriber must receive every event despite an unresponsive peer"
                );

                // _slow_stream and _slow_seed are dropped here (end of scope
                // after cancel), keeping the slow connection open for the whole
                // flood as required.
                cancel.cancel();
                serve_join.await.expect("serve_http join");
            })
            .await;

            result.expect("test timed out");
        }

        /// POST `env`, drain its submission, and emit its `[Started, Completed]`
        /// pair. Helper for the high-volume slow-consumer test.
        async fn handle_submit_and_emit(
            addr: std::net::SocketAddr,
            sub_rx: &mut mpsc::UnboundedReceiver<OpEnvelope>,
            evt_tx: &mpsc::UnboundedSender<EventMsg>,
            env: &OpEnvelope,
        ) {
            post_op(addr, env).await;
            let received = sub_rx.recv().await.expect("op received");
            assert_eq!(received.op_id, env.op_id);
            emit_started_completed(evt_tx, env);
        }
    }

    #[test]
    fn validates_daemon_endpoints() {
        assert!(
            validate_endpoint(&DaemonEndpoint {
                transport: DaemonTransport::Tcp,
                address: "127.0.0.1:8787".to_string(),
            })
            .is_ok()
        );
        assert!(
            validate_endpoint(&DaemonEndpoint {
                transport: DaemonTransport::Tcp,
                address: "tdw-service-daemon:7878".to_string(),
            })
            .is_ok()
        );
        assert_eq!(
            validate_endpoint(&DaemonEndpoint {
                transport: DaemonTransport::Tcp,
                address: "localhost:not-a-port".to_string(),
            }),
            Err(EndpointError::InvalidTcpAddress)
        );
        assert!(
            validate_endpoint(&DaemonEndpoint {
                transport: DaemonTransport::Uds,
                address: "~/.tdw/daemon.sock".to_string(),
            })
            .is_ok()
        );
        assert_eq!(
            validate_endpoint(&DaemonEndpoint {
                transport: DaemonTransport::HttpSse,
                address: "file:///tmp/socket".to_string(),
            }),
            Err(EndpointError::InvalidHttpSseAddress)
        );
        assert_eq!(
            validate_endpoint(&DaemonEndpoint {
                transport: DaemonTransport::Uds,
                address: "../daemon.sock".to_string(),
            }),
            Err(EndpointError::InvalidUdsAddress)
        );
    }
}
