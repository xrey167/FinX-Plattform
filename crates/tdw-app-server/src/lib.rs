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
