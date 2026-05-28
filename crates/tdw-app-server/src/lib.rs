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
