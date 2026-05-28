#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tdw_app_server::{
    DaemonEndpoint, DaemonTransport, EndpointError, SubmissionError, SubmissionHandle,
};
use tdw_protocol::{EventMsg, OpEnvelope, OpId};

pub const DEFAULT_DAEMON_TCP_ADDR: &str = "127.0.0.1:7878";

const DEFAULT_DAEMON_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_DAEMON_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_DAEMON_EVENTS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub endpoint: DaemonEndpoint,
}

#[derive(Clone)]
pub struct AppClient {
    info: ClientInfo,
    submissions: SubmissionHandle,
}

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientError {
    InvalidName,
    InvalidEndpoint(EndpointError),
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => write!(formatter, "client name is invalid"),
            Self::InvalidEndpoint(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for ClientError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonClientConfig {
    endpoint: DaemonEndpoint,
    timeout: Duration,
}

impl Default for DaemonClientConfig {
    fn default() -> Self {
        Self::tcp(DEFAULT_DAEMON_TCP_ADDR)
    }
}

impl DaemonClientConfig {
    pub fn tcp(address: impl Into<String>) -> Self {
        Self {
            endpoint: DaemonEndpoint {
                transport: DaemonTransport::Tcp,
                address: address.into(),
            },
            timeout: DEFAULT_DAEMON_TIMEOUT,
        }
    }

    pub fn new(endpoint: DaemonEndpoint) -> Self {
        Self {
            endpoint,
            timeout: DEFAULT_DAEMON_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        if !timeout.is_zero() {
            self.timeout = timeout;
        }
        self
    }

    pub fn endpoint(&self) -> &DaemonEndpoint {
        &self.endpoint
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub fn validate(&self) -> std::result::Result<(), DaemonClientError> {
        self.endpoint
            .validate()
            .map_err(DaemonClientError::InvalidEndpoint)?;
        match self.endpoint.transport {
            DaemonTransport::Tcp => Ok(()),
            transport => Err(DaemonClientError::UnsupportedTransport(transport)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DaemonClient {
    config: DaemonClientConfig,
}

impl Default for DaemonClient {
    fn default() -> Self {
        Self::new(DaemonClientConfig::default())
    }
}

impl DaemonClient {
    pub fn new(config: DaemonClientConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &DaemonClientConfig {
        &self.config
    }

    pub fn submit_and_wait(&self, envelope: OpEnvelope) -> DaemonClientResult {
        self.config.validate()?;
        match self.config.endpoint.transport {
            DaemonTransport::Tcp => self.submit_tcp(envelope),
            transport => Err(DaemonClientError::UnsupportedTransport(transport)),
        }
    }

    fn submit_tcp(&self, envelope: OpEnvelope) -> DaemonClientResult {
        let op_id = envelope.op_id.clone();
        let address = self.config.endpoint.address.clone();
        let mut stream =
            TcpStream::connect(&address).map_err(|source| DaemonClientError::Connect {
                address: address.clone(),
                source,
            })?;
        stream
            .set_read_timeout(Some(self.config.timeout))
            .map_err(|source| DaemonClientError::Io {
                action: "set read timeout",
                source,
            })?;
        stream
            .set_write_timeout(Some(self.config.timeout))
            .map_err(|source| DaemonClientError::Io {
                action: "set write timeout",
                source,
            })?;

        write_envelope_frame(&mut stream, &envelope)?;

        let mut events = Vec::new();
        for _ in 0..MAX_DAEMON_EVENTS {
            let frame = match read_event_frame(&mut stream) {
                Ok(frame) => frame,
                Err(DaemonClientError::Io { action, source }) if is_timeout(&source) => {
                    return Err(DaemonClientError::TimedOut {
                        op_id: op_id.as_str().to_string(),
                        action,
                    });
                }
                Err(error) => return Err(error),
            };
            let event: EventMsg =
                serde_json::from_slice(&frame).map_err(DaemonClientError::Deserialize)?;
            let is_matching_terminal =
                event_op_id(&event) == Some(&op_id) && event_is_terminal(&event);
            events.push(event);
            if is_matching_terminal {
                return Ok(DaemonSubmission {
                    endpoint: self.config.endpoint.clone(),
                    op_id: op_id.as_str().to_string(),
                    events,
                });
            }
        }

        Err(DaemonClientError::TerminalEventMissing {
            op_id: op_id.as_str().to_string(),
            events_seen: events.len(),
        })
    }
}

pub type DaemonClientResult = std::result::Result<DaemonSubmission, DaemonClientError>;

#[derive(Clone, Debug, PartialEq)]
pub struct DaemonSubmission {
    pub endpoint: DaemonEndpoint,
    pub op_id: String,
    pub events: Vec<EventMsg>,
}

#[derive(Debug)]
pub enum DaemonClientError {
    InvalidEndpoint(EndpointError),
    UnsupportedTransport(DaemonTransport),
    Connect {
        address: String,
        source: std::io::Error,
    },
    Io {
        action: &'static str,
        source: std::io::Error,
    },
    Serialize(serde_json::Error),
    Deserialize(serde_json::Error),
    EmptyFrame,
    FrameTooLarge {
        bytes: usize,
    },
    TimedOut {
        op_id: String,
        action: &'static str,
    },
    TerminalEventMissing {
        op_id: String,
        events_seen: usize,
    },
}

impl fmt::Display for DaemonClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEndpoint(error) => write!(formatter, "{error}"),
            Self::UnsupportedTransport(transport) => {
                write!(
                    formatter,
                    "unsupported daemon client transport: {transport:?}"
                )
            }
            Self::Connect { address, source } => {
                write!(formatter, "daemon unavailable at {address}: {source}")
            }
            Self::Io { action, source } => write!(formatter, "daemon {action} failed: {source}"),
            Self::Serialize(error) => {
                write!(formatter, "daemon envelope serialization failed: {error}")
            }
            Self::Deserialize(error) => {
                write!(formatter, "daemon event deserialization failed: {error}")
            }
            Self::EmptyFrame => write!(formatter, "daemon returned an empty frame"),
            Self::FrameTooLarge { bytes } => {
                write!(formatter, "daemon frame is too large: {bytes} bytes")
            }
            Self::TimedOut { op_id, action } => {
                write!(formatter, "daemon timed out during {action} for op {op_id}")
            }
            Self::TerminalEventMissing { op_id, events_seen } => write!(
                formatter,
                "daemon did not emit a terminal event for op {op_id} after {events_seen} events"
            ),
        }
    }
}

impl Error for DaemonClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidEndpoint(error) => Some(error),
            Self::Connect { source, .. } | Self::Io { source, .. } => Some(source),
            Self::Serialize(error) | Self::Deserialize(error) => Some(error),
            _ => None,
        }
    }
}

fn write_envelope_frame(
    stream: &mut TcpStream,
    envelope: &OpEnvelope,
) -> std::result::Result<(), DaemonClientError> {
    let bytes = serde_json::to_vec(envelope).map_err(DaemonClientError::Serialize)?;
    let len = u32::try_from(bytes.len())
        .map_err(|_| DaemonClientError::FrameTooLarge { bytes: bytes.len() })?;
    stream
        .write_all(&len.to_be_bytes())
        .map_err(|source| DaemonClientError::Io {
            action: "write frame length",
            source,
        })?;
    stream
        .write_all(&bytes)
        .map_err(|source| DaemonClientError::Io {
            action: "write frame body",
            source,
        })?;
    stream.flush().map_err(|source| DaemonClientError::Io {
        action: "flush frame",
        source,
    })
}

fn read_event_frame(stream: &mut TcpStream) -> std::result::Result<Vec<u8>, DaemonClientError> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(|source| DaemonClientError::Io {
            action: "read frame length",
            source,
        })?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Err(DaemonClientError::EmptyFrame);
    }
    if len > MAX_DAEMON_FRAME_BYTES {
        return Err(DaemonClientError::FrameTooLarge { bytes: len });
    }
    let mut bytes = vec![0u8; len];
    stream
        .read_exact(&mut bytes)
        .map_err(|source| DaemonClientError::Io {
            action: "read frame body",
            source,
        })?;
    Ok(bytes)
}

fn event_op_id(event: &EventMsg) -> Option<&OpId> {
    match event {
        EventMsg::Started { op_id }
        | EventMsg::Progress { op_id, .. }
        | EventMsg::ToolCallRequested { op_id, .. }
        | EventMsg::ToolCallCompleted { op_id, .. }
        | EventMsg::OutputChunk { op_id, .. }
        | EventMsg::DomainEvent { op_id, .. }
        | EventMsg::Completed { op_id, .. }
        | EventMsg::Failed { op_id, .. }
        | EventMsg::Cancelled { op_id, .. } => Some(op_id),
        EventMsg::ApprovalRequested { .. } => None,
    }
}

fn event_is_terminal(event: &EventMsg) -> bool {
    matches!(
        event,
        EventMsg::Completed { .. } | EventMsg::Failed { .. } | EventMsg::Cancelled { .. }
    )
}

fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

pub fn validate_client_info(info: &ClientInfo) -> Result<()> {
    let name = info.name.trim();
    if name.is_empty()
        || name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ';' | '|' | '&'))
        || name.contains("..")
    {
        return Err(ClientError::InvalidName);
    }

    info.endpoint
        .validate()
        .map_err(ClientError::InvalidEndpoint)
}

impl AppClient {
    pub fn new(info: ClientInfo, submissions: SubmissionHandle) -> Self {
        Self { info, submissions }
    }

    pub fn try_new(info: ClientInfo, submissions: SubmissionHandle) -> Result<Self> {
        validate_client_info(&info)?;
        Ok(Self::new(info, submissions))
    }

    pub fn info(&self) -> &ClientInfo {
        &self.info
    }

    pub fn submit(&self, envelope: OpEnvelope) -> std::result::Result<(), SubmissionError> {
        self.submissions.submit(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_app_server::{DaemonTransport, channel};
    use tdw_protocol::{ActorKind, ActorRef, Op, SessionId};

    #[test]
    fn client_submits_to_shared_daemon_handle() {
        let (handle, _events, _loop_runner) = channel();
        let client = AppClient::try_new(
            ClientInfo {
                name: "tdw-cli".to_string(),
                endpoint: DaemonEndpoint {
                    transport: DaemonTransport::Uds,
                    address: "~/.tdw/daemon.sock".to_string(),
                },
            },
            handle,
        )
        .expect("valid client");
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

        assert!(client.submit(envelope).is_ok());
        assert_eq!(client.info().endpoint.transport, DaemonTransport::Uds);
    }

    #[test]
    fn rejects_invalid_client_identity_or_endpoint() {
        let (handle, _events, _loop_runner) = channel();
        let result = AppClient::try_new(
            ClientInfo {
                name: "../cli".to_string(),
                endpoint: DaemonEndpoint {
                    transport: DaemonTransport::HttpSse,
                    address: "https://localhost:8787/events".to_string(),
                },
            },
            handle,
        );

        assert!(matches!(result, Err(ClientError::InvalidName)));
    }

    #[test]
    fn daemon_client_config_defaults_to_local_tcp_and_rejects_unsupported_transport() {
        let config = DaemonClientConfig::default();
        assert_eq!(config.endpoint().transport, DaemonTransport::Tcp);
        assert_eq!(config.endpoint().address, DEFAULT_DAEMON_TCP_ADDR);
        assert!(config.validate().is_ok());

        let unsupported = DaemonClientConfig::new(DaemonEndpoint {
            transport: DaemonTransport::HttpSse,
            address: "http://127.0.0.1:7879/events".to_string(),
        });
        assert!(matches!(
            unsupported.validate(),
            Err(DaemonClientError::UnsupportedTransport(
                DaemonTransport::HttpSse
            ))
        ));
    }
}
