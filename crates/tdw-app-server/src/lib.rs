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

#[cfg(feature = "transport-tcp")]
mod transport_tcp;
#[cfg(feature = "transport-tcp")]
pub use transport_tcp::serve_tcp;

#[cfg(all(unix, feature = "transport-uds"))]
mod transport_uds;
#[cfg(all(unix, feature = "transport-uds"))]
pub use transport_uds::serve_uds;

#[cfg(feature = "transport-http")]
mod transport_http;
#[cfg(feature = "transport-http")]
pub use transport_http::serve_http;

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
    address
        .parse::<SocketAddr>()
        .map(|_| ())
        .map_err(|_| EndpointError::InvalidTcpAddress)
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

/// Returns a human-readable label for the transport variant (useful for logging).
#[must_use]
pub const fn transport_label(t: DaemonTransport) -> &'static str {
    match t {
        DaemonTransport::Tcp => "tcp",
        DaemonTransport::Uds => "uds",
        DaemonTransport::HttpSse => "http-sse",
    }
}

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
            () = shutdown.cancelled() => break,
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

    // -----------------------------------------------------------------------
    // P4 transport tests
    // -----------------------------------------------------------------------

    #[cfg(feature = "transport-tcp")]
    mod tcp_transport_tests {
        use super::*;
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::{TcpListener, TcpStream};

        async fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> std::io::Result<()> {
            let len = (bytes.len() as u32).to_be_bytes();
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
