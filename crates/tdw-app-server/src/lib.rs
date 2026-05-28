#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use tdw_protocol::{EventMsg, OpEnvelope};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaemonTransport {
    Uds,
    HttpSse,
}

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
    InvalidHttpSseAddress,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAddress => write!(formatter, "daemon endpoint address must not be empty"),
            Self::InvalidUdsAddress => write!(formatter, "invalid UDS daemon endpoint address"),
            Self::InvalidHttpSseAddress => {
                write!(formatter, "invalid HTTP/SSE daemon endpoint address")
            }
        }
    }
}

impl Error for EndpointError {}

impl DaemonEndpoint {
    pub fn validate(&self) -> EndpointResult<()> {
        validate_endpoint(self)
    }
}

pub fn validate_endpoint(endpoint: &DaemonEndpoint) -> EndpointResult<()> {
    let address = endpoint.address.trim();
    if address.is_empty() {
        return Err(EndpointError::EmptyAddress);
    }
    if address.chars().any(char::is_control) {
        return Err(match endpoint.transport {
            DaemonTransport::Uds => EndpointError::InvalidUdsAddress,
            DaemonTransport::HttpSse => EndpointError::InvalidHttpSseAddress,
        });
    }

    match endpoint.transport {
        DaemonTransport::Uds => validate_uds_address(address),
        DaemonTransport::HttpSse => validate_http_sse_address(address),
    }
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
    pub fn into_envelope(self) -> OpEnvelope {
        *self.envelope
    }
}

impl SubmissionHandle {
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
}

impl<D: Dispatcher + 'static, S: EventSink + 'static> ServiceLoop<D, S> {
    pub fn new(
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
        }
    }

    /// Receive one envelope, dispatch it, persist each event, send events on
    /// the channel, then record cost. Returns the events emitted, or `None` if
    /// the submission channel is closed.
    pub async fn run_once(&mut self) -> Option<Vec<EventMsg>> {
        let env = self.submissions.recv().await?;
        let emitted = self.dispatcher.dispatch(env.clone()).await;
        for event in &emitted {
            let seq = self
                .next_sequence
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Err(e) = self.sink.persist_event(&env, event, seq).await {
                eprintln!("[ServiceLoop] persist_event error (seq={seq}): {e}");
            }
            let _ = self.events.send(event.clone());
        }
        if let Err(e) = self.sink.record_cost(&env, "in-memory").await {
            eprintln!("[ServiceLoop] record_cost error: {e}");
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

/// Spawn the in-memory outbox→bus relay. The task polls the outbox for
/// pending records every `tick`, publishes each on the bus, and marks it
/// dispatched. Cooperative shutdown via the supplied `CancellationToken`.
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
                let guard = outbox.lock().expect("outbox lock");
                guard.pending_after(last_seq)
            };
            for record in &pending {
                // Publish a fresh EventEnvelope of the same payload onto the bus.
                let envelope = record.envelope.clone();
                {
                    let mut bus_guard = bus.lock().expect("bus lock");
                    bus_guard.publish(envelope);
                }
                {
                    let mut outbox_guard = outbox.lock().expect("outbox lock");
                    outbox_guard.mark_dispatched(record.sequence);
                }
                if record.sequence > last_seq {
                    last_seq = record.sequence;
                }
            }
            // Wait for next tick or cancellation.
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(tick) => {}
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
pub async fn serve<D: Dispatcher + 'static, S: EventSink + 'static>(
    mut service_loop: ServiceLoop<D, S>,
    relay: tokio::task::JoinHandle<()>,
    shutdown: tokio_util::sync::CancellationToken,
) -> std::result::Result<(), SinkError> {
    let shutdown_for_signal = shutdown.clone();
    let signal_task: tokio::task::JoinHandle<()> = tokio::spawn(async move {
        // Ignore signal install errors on platforms that don't support ctrl_c.
        if tokio::signal::ctrl_c().await.is_ok() {
            shutdown_for_signal.cancel();
        }
    });

    loop {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
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

    // Drain: ensure relay task observes cancellation, then await both.
    shutdown.cancel();
    let _ = relay.await;
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

    #[test]
    fn validates_daemon_endpoints() {
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
