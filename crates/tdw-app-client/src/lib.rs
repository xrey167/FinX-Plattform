#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

use serde::{Deserialize, Serialize};
use tdw_app_server::{
    DaemonEndpoint, DaemonTransport, EndpointError, SubmissionError, SubmissionHandle,
};
use tdw_protocol::{EventMsg, OpEnvelope, OpId};

pub const DEFAULT_DAEMON_TCP_ADDR: &str = "127.0.0.1:7878";

const DEFAULT_DAEMON_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_HTTP_HEADER_BYTES: usize = 8 * 1024;
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
    #[must_use]
    pub fn tcp(address: impl Into<String>) -> Self {
        Self {
            endpoint: DaemonEndpoint {
                transport: DaemonTransport::Tcp,
                address: address.into(),
            },
            timeout: DEFAULT_DAEMON_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn new(endpoint: DaemonEndpoint) -> Self {
        Self {
            endpoint,
            timeout: DEFAULT_DAEMON_TIMEOUT,
        }
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        if !timeout.is_zero() {
            self.timeout = timeout;
        }
        self
    }

    #[must_use]
    pub const fn endpoint(&self) -> &DaemonEndpoint {
        &self.endpoint
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    /// # Errors
    ///
    /// Returns `DaemonClientError::InvalidEndpoint` if the endpoint fails
    /// validation, or `DaemonClientError::UnsupportedTransport` if a Unix
    /// domain socket transport is requested on a non-Unix platform.
    pub fn validate(&self) -> std::result::Result<(), DaemonClientError> {
        self.endpoint
            .validate()
            .map_err(DaemonClientError::InvalidEndpoint)?;
        match self.endpoint.transport {
            DaemonTransport::Tcp => Ok(()),
            #[cfg(unix)]
            DaemonTransport::Uds => Ok(()),
            #[cfg(not(unix))]
            DaemonTransport::Uds => Err(DaemonClientError::UnsupportedTransport(
                DaemonTransport::Uds,
            )),
            DaemonTransport::HttpSse => Ok(()),
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
    #[must_use]
    pub const fn new(config: DaemonClientConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(&self) -> &DaemonClientConfig {
        &self.config
    }

    /// # Errors
    ///
    /// Returns a `DaemonClientError` if config validation fails or the
    /// underlying transport request fails.
    pub fn submit_and_wait(&self, envelope: &OpEnvelope) -> DaemonClientResult {
        self.config.validate()?;
        match self.config.endpoint.transport {
            DaemonTransport::Tcp => self.submit_tcp(envelope),
            #[cfg(unix)]
            DaemonTransport::Uds => self.submit_uds(envelope),
            #[cfg(not(unix))]
            DaemonTransport::Uds => Err(DaemonClientError::UnsupportedTransport(
                DaemonTransport::Uds,
            )),
            DaemonTransport::HttpSse => self.submit_http_sse(envelope),
        }
    }

    fn submit_tcp(&self, envelope: &OpEnvelope) -> DaemonClientResult {
        let op_id = envelope.op_id.clone();
        let address = self.config.endpoint.address.trim().to_string();
        let socket_addr = parse_tcp_socket_addr(&address)?;
        let mut stream = connect_tcp_stream(socket_addr, &address, self.config.timeout, &op_id)?;
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

        write_envelope_frame(&mut stream, envelope)?;
        let events = read_terminal_events(&mut stream, &op_id)?;
        Ok(DaemonSubmission {
            endpoint: self.config.endpoint.clone(),
            op_id: op_id.as_str().to_string(),
            events,
        })
    }

    #[cfg(unix)]
    fn submit_uds(&self, envelope: &OpEnvelope) -> DaemonClientResult {
        let op_id = envelope.op_id.clone();
        let address = self.config.endpoint.address.trim().to_string();
        let mut stream =
            UnixStream::connect(&address).map_err(|source| DaemonClientError::Connect {
                address: address.clone(),
                source,
            })?;
        stream
            .set_read_timeout(Some(self.config.timeout))
            .map_err(|source| DaemonClientError::Io {
                action: "set UDS read timeout",
                source,
            })?;
        stream
            .set_write_timeout(Some(self.config.timeout))
            .map_err(|source| DaemonClientError::Io {
                action: "set UDS write timeout",
                source,
            })?;

        write_envelope_frame(&mut stream, envelope)?;
        let events = read_terminal_events(&mut stream, &op_id)?;
        Ok(DaemonSubmission {
            endpoint: self.config.endpoint.clone(),
            op_id: op_id.as_str().to_string(),
            events,
        })
    }

    fn submit_http_sse(&self, envelope: &OpEnvelope) -> DaemonClientResult {
        let op_id = envelope.op_id.clone();
        let endpoint = parse_http_sse_endpoint(self.config.endpoint.address.trim())?;

        let mut events_stream = connect_tcp_authority(
            &endpoint.authority,
            self.config.timeout,
            &op_id,
            "connect http/sse events",
        )?;
        set_tcp_timeouts(&events_stream, self.config.timeout, "http/sse events")?;
        let events_request = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
            endpoint.events_path, endpoint.authority
        );
        events_stream
            .write_all(events_request.as_bytes())
            .map_err(|source| DaemonClientError::Io {
                action: "write http/sse events request",
                source,
            })?;
        let events_headers = read_http_headers(&mut events_stream, "read http/sse events headers")?;
        expect_http_status(
            &events_headers,
            200,
            "open http/sse events stream",
            "text/event-stream",
        )?;

        let mut submit_stream = connect_tcp_authority(
            &endpoint.authority,
            self.config.timeout,
            &op_id,
            "connect http/sse submit",
        )?;
        set_tcp_timeouts(&submit_stream, self.config.timeout, "http/sse submit")?;
        let body = serde_json::to_vec(envelope).map_err(DaemonClientError::Serialize)?;
        let submit_request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            endpoint.submit_path,
            endpoint.authority,
            body.len()
        );
        submit_stream
            .write_all(submit_request.as_bytes())
            .map_err(|source| DaemonClientError::Io {
                action: "write http/sse submit headers",
                source,
            })?;
        submit_stream
            .write_all(&body)
            .map_err(|source| DaemonClientError::Io {
                action: "write http/sse submit body",
                source,
            })?;
        submit_stream
            .flush()
            .map_err(|source| DaemonClientError::Io {
                action: "flush http/sse submit",
                source,
            })?;
        let submit_headers = read_http_headers(&mut submit_stream, "read http/sse submit headers")?;
        expect_http_status(&submit_headers, 202, "submit http/sse op", "")?;

        let events = read_terminal_events(&mut HttpSseEventReader::new(events_stream), &op_id)?;
        Ok(DaemonSubmission {
            endpoint: self.config.endpoint.clone(),
            op_id: op_id.as_str().to_string(),
            events,
        })
    }
}

struct HttpSseEndpoint {
    authority: String,
    events_path: String,
    submit_path: String,
}

fn parse_http_sse_endpoint(
    address: &str,
) -> std::result::Result<HttpSseEndpoint, DaemonClientError> {
    let Some(rest) = address.strip_prefix("http://") else {
        return Err(DaemonClientError::UnsupportedEndpoint {
            transport: DaemonTransport::HttpSse,
            reason: "only plain http:// daemon endpoints are supported",
        });
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, "events"));
    if authority.is_empty() {
        return Err(DaemonClientError::InvalidEndpoint(
            EndpointError::InvalidHttpSseAddress,
        ));
    }

    let path = if path.trim_matches('/').is_empty() {
        "events"
    } else {
        path
    };
    let events_path = format!("/{}", path.trim_start_matches('/'));
    let submit_path = events_path.strip_suffix("/events").map_or_else(
        || "/op".to_string(),
        |prefix| {
            if prefix.is_empty() {
                "/op".to_string()
            } else {
                format!("{prefix}/op")
            }
        },
    );

    Ok(HttpSseEndpoint {
        authority: authority.to_string(),
        events_path,
        submit_path,
    })
}

fn parse_tcp_socket_addr(address: &str) -> std::result::Result<SocketAddr, DaemonClientError> {
    address
        .parse()
        .map_err(|_| DaemonClientError::InvalidEndpoint(EndpointError::InvalidTcpAddress))
}

fn connect_tcp_stream(
    socket_addr: SocketAddr,
    address: &str,
    timeout: Duration,
    op_id: &OpId,
) -> std::result::Result<TcpStream, DaemonClientError> {
    TcpStream::connect_timeout(&socket_addr, timeout).map_err(|source| {
        if is_timeout(&source) {
            DaemonClientError::TimedOut {
                op_id: op_id.as_str().to_string(),
                action: "connect",
            }
        } else {
            DaemonClientError::Connect {
                address: address.to_string(),
                source,
            }
        }
    })
}

fn connect_tcp_authority(
    authority: &str,
    timeout: Duration,
    op_id: &OpId,
    action: &'static str,
) -> std::result::Result<TcpStream, DaemonClientError> {
    let mut last_error = None;
    let socket_addrs =
        authority
            .to_socket_addrs()
            .map_err(|source| DaemonClientError::Connect {
                address: authority.to_string(),
                source,
            })?;

    for socket_addr in socket_addrs {
        match TcpStream::connect_timeout(&socket_addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(source) if is_timeout(&source) => {
                return Err(DaemonClientError::TimedOut {
                    op_id: op_id.as_str().to_string(),
                    action,
                });
            }
            Err(source) => last_error = Some(source),
        }
    }

    Err(DaemonClientError::Connect {
        address: authority.to_string(),
        source: last_error.unwrap_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "daemon endpoint resolved to no socket addresses",
            )
        }),
    })
}

fn set_tcp_timeouts(
    stream: &TcpStream,
    timeout: Duration,
    label: &'static str,
) -> std::result::Result<(), DaemonClientError> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|source| DaemonClientError::Io {
            action: match label {
                "http/sse events" => "set http/sse events read timeout",
                "http/sse submit" => "set http/sse submit read timeout",
                _ => "set read timeout",
            },
            source,
        })?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|source| DaemonClientError::Io {
            action: match label {
                "http/sse events" => "set http/sse events write timeout",
                "http/sse submit" => "set http/sse submit write timeout",
                _ => "set write timeout",
            },
            source,
        })
}

fn read_http_headers(
    stream: &mut TcpStream,
    action: &'static str,
) -> std::result::Result<String, DaemonClientError> {
    let mut headers = Vec::new();
    let mut byte = [0u8; 1];
    while headers.len() < MAX_HTTP_HEADER_BYTES {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                headers.push(byte[0]);
                if headers.ends_with(b"\r\n\r\n") {
                    return String::from_utf8(headers).map_err(|error| {
                        DaemonClientError::Protocol {
                            action,
                            message: format!("HTTP headers were not UTF-8: {error}"),
                        }
                    });
                }
            }
            Err(source) if is_timeout(&source) => {
                return Err(DaemonClientError::TimedOut {
                    op_id: "unknown".to_string(),
                    action,
                });
            }
            Err(source) => return Err(DaemonClientError::Io { action, source }),
        }
    }

    Err(DaemonClientError::Protocol {
        action,
        message: "HTTP response headers were incomplete or too large".to_string(),
    })
}

fn expect_http_status(
    headers: &str,
    expected: u16,
    action: &'static str,
    required_content_type: &str,
) -> std::result::Result<(), DaemonClientError> {
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| DaemonClientError::Protocol {
            action,
            message: format!("HTTP response did not include a status code: {headers:?}"),
        })?;

    if status != expected {
        return Err(DaemonClientError::Protocol {
            action,
            message: format!("expected HTTP {expected}, got HTTP {status}"),
        });
    }

    if !required_content_type.is_empty()
        && !headers
            .lines()
            .any(|line| line.to_ascii_lowercase().contains(required_content_type))
    {
        return Err(DaemonClientError::Protocol {
            action,
            message: format!("HTTP response did not include {required_content_type}"),
        });
    }

    Ok(())
}

trait EventFrameReader {
    fn read_event_frame(&mut self) -> std::result::Result<Vec<u8>, DaemonClientError>;
}

impl<T: Read> EventFrameReader for T {
    fn read_event_frame(&mut self) -> std::result::Result<Vec<u8>, DaemonClientError> {
        read_length_delimited_event_frame(self)
    }
}

struct HttpSseEventReader {
    stream: TcpStream,
}

impl HttpSseEventReader {
    const fn new(stream: TcpStream) -> Self {
        Self { stream }
    }
}

impl EventFrameReader for HttpSseEventReader {
    fn read_event_frame(&mut self) -> std::result::Result<Vec<u8>, DaemonClientError> {
        let mut frame = Vec::new();
        let mut byte = [0u8; 1];
        while frame.len() < MAX_DAEMON_FRAME_BYTES {
            match self.stream.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    frame.push(byte[0]);
                    if frame.ends_with(b"\n\n") || frame.ends_with(b"\r\n\r\n") {
                        let text = std::str::from_utf8(&frame).map_err(|error| {
                            DaemonClientError::Protocol {
                                action: "read http/sse event",
                                message: format!("SSE event was not UTF-8: {error}"),
                            }
                        })?;
                        let data = text
                            .lines()
                            .filter_map(|line| line.strip_prefix("data:"))
                            .map(str::trim_start)
                            .collect::<Vec<_>>()
                            .join("\n");
                        if data.is_empty() {
                            return Err(DaemonClientError::Protocol {
                                action: "read http/sse event",
                                message: "SSE event had no data field".to_string(),
                            });
                        }
                        return Ok(data.into_bytes());
                    }
                }
                Err(source) if is_timeout(&source) => {
                    return Err(DaemonClientError::TimedOut {
                        op_id: "unknown".to_string(),
                        action: "read http/sse event",
                    });
                }
                Err(source) => {
                    return Err(DaemonClientError::Io {
                        action: "read http/sse event",
                        source,
                    });
                }
            }
        }

        Err(DaemonClientError::Protocol {
            action: "read http/sse event",
            message: "SSE event frame was incomplete or too large".to_string(),
        })
    }
}

fn read_terminal_events(
    reader: &mut impl EventFrameReader,
    op_id: &OpId,
) -> std::result::Result<Vec<EventMsg>, DaemonClientError> {
    let mut events = Vec::new();
    for _ in 0..MAX_DAEMON_EVENTS {
        let frame = match reader.read_event_frame() {
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
        let is_matching_terminal = event_op_id(&event) == Some(op_id) && event_is_terminal(&event);
        events.push(event);
        if is_matching_terminal {
            return Ok(events);
        }
    }

    Err(DaemonClientError::TerminalEventMissing {
        op_id: op_id.as_str().to_string(),
        events_seen: events.len(),
    })
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
    UnsupportedEndpoint {
        transport: DaemonTransport,
        reason: &'static str,
    },
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
    Protocol {
        action: &'static str,
        message: String,
    },
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
            Self::UnsupportedEndpoint { transport, reason } => {
                write!(
                    formatter,
                    "unsupported daemon client endpoint for {transport:?}: {reason}"
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
            Self::Protocol { action, message } => {
                write!(formatter, "daemon {action} protocol error: {message}")
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
    stream: &mut impl Write,
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

fn read_length_delimited_event_frame(
    stream: &mut impl Read,
) -> std::result::Result<Vec<u8>, DaemonClientError> {
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

/// Fuzz shim: feed arbitrary bytes through the daemon frame reader.
///
/// Must never panic on adversarial input; truncated, oversized, or empty
/// frames must surface as errors rather than panics. Shared with the nightly
/// cargo-fuzz target. Lives in-crate to reach the `pub(crate)` reader.
#[doc(hidden)]
pub fn __fuzz_daemon_frame(data: &[u8]) {
    let mut cursor = std::io::Cursor::new(data);
    let _ = read_length_delimited_event_frame(&mut cursor);
}

const fn event_op_id(event: &EventMsg) -> Option<&OpId> {
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

const fn event_is_terminal(event: &EventMsg) -> bool {
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

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
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
    #[must_use]
    pub const fn new(info: ClientInfo, submissions: SubmissionHandle) -> Self {
        Self { info, submissions }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_new(info: ClientInfo, submissions: SubmissionHandle) -> Result<Self> {
        validate_client_info(&info)?;
        Ok(Self::new(info, submissions))
    }

    #[must_use]
    pub const fn info(&self) -> &ClientInfo {
        &self.info
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn submit(&self, envelope: OpEnvelope) -> std::result::Result<(), SubmissionError> {
        self.submissions.submit(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;
    use tdw_app_server::{DaemonTransport, channel};
    use tdw_protocol::{ActorKind, ActorRef, Op, SessionId};

    fn shutdown_envelope() -> OpEnvelope {
        OpEnvelope::new(
            SessionId::new("session-1").expect("session id"),
            1,
            ActorRef {
                actor_id: "user".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::Shutdown,
        )
    }

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
        let envelope = shutdown_envelope();

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
    fn daemon_client_config_defaults_to_local_tcp_and_validates_supported_transports() {
        let config = DaemonClientConfig::default();
        assert_eq!(config.endpoint().transport, DaemonTransport::Tcp);
        assert_eq!(config.endpoint().address, DEFAULT_DAEMON_TCP_ADDR);
        assert_eq!(config.timeout(), DEFAULT_DAEMON_TIMEOUT);
        assert!(config.validate().is_ok());

        assert_eq!(
            DaemonClientConfig::default()
                .with_timeout(Duration::ZERO)
                .timeout(),
            DEFAULT_DAEMON_TIMEOUT
        );
        assert_eq!(
            DaemonClientConfig::default()
                .with_timeout(Duration::from_millis(25))
                .timeout(),
            Duration::from_millis(25)
        );

        let http_sse = DaemonClientConfig::new(DaemonEndpoint {
            transport: DaemonTransport::HttpSse,
            address: "http://127.0.0.1:7879/events".to_string(),
        });
        assert!(http_sse.validate().is_ok());

        let uds = DaemonClientConfig::new(DaemonEndpoint {
            transport: DaemonTransport::Uds,
            address: "/tmp/tdw.sock".to_string(),
        });
        #[cfg(unix)]
        assert!(uds.validate().is_ok());
        #[cfg(not(unix))]
        assert!(matches!(
            uds.validate(),
            Err(DaemonClientError::UnsupportedTransport(
                DaemonTransport::Uds
            ))
        ));
    }

    #[test]
    fn daemon_client_reports_bounded_tcp_connect_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral listener");
        let addr = listener.local_addr().expect("local address");
        drop(listener);

        let client = DaemonClient::new(
            DaemonClientConfig::tcp(addr.to_string()).with_timeout(Duration::from_millis(10)),
        );

        match client
            .submit_and_wait(&shutdown_envelope())
            .expect_err("closed daemon port should not accept submissions")
        {
            DaemonClientError::Connect { address, .. } => {
                assert_eq!(address, addr.to_string());
            }
            DaemonClientError::TimedOut { action, .. } => {
                assert_eq!(action, "connect");
            }
            error => panic!("unexpected daemon client error: {error}"),
        }
    }

    #[test]
    fn daemon_client_fails_closed_for_unsupported_https_sse_endpoint() {
        let client = DaemonClient::new(DaemonClientConfig::new(DaemonEndpoint {
            transport: DaemonTransport::HttpSse,
            address: "https://127.0.0.1:7879/events".to_string(),
        }));

        assert!(matches!(
            client.submit_and_wait(&shutdown_envelope()),
            Err(DaemonClientError::UnsupportedEndpoint {
                transport: DaemonTransport::HttpSse,
                ..
            })
        ));
    }

    #[test]
    fn daemon_envelope_frame_uses_big_endian_length_prefix() {
        let envelope = shutdown_envelope();
        let mut frame = Vec::new();

        write_envelope_frame(&mut frame, &envelope).expect("write envelope frame");

        assert!(frame.len() > 4);
        let len = u32::from_be_bytes(
            frame[0..4]
                .try_into()
                .expect("frame length prefix should be four bytes"),
        ) as usize;
        assert_eq!(len, frame.len() - 4);
        let decoded: OpEnvelope =
            serde_json::from_slice(&frame[4..]).expect("frame body should be an OpEnvelope");
        assert_eq!(decoded.op_id, envelope.op_id);
        assert_eq!(decoded.session_id, envelope.session_id);
    }

    #[test]
    fn daemon_event_frame_reader_rejects_empty_and_oversized_frames() {
        let mut empty = Cursor::new([0_u8, 0, 0, 0]);
        assert!(matches!(
            read_length_delimited_event_frame(&mut empty),
            Err(DaemonClientError::EmptyFrame)
        ));

        let oversized_len = u32::try_from(MAX_DAEMON_FRAME_BYTES + 1)
            .expect("oversized test length fits u32")
            .to_be_bytes();
        let mut oversized = Cursor::new(oversized_len);
        assert!(matches!(
            read_length_delimited_event_frame(&mut oversized),
            Err(DaemonClientError::FrameTooLarge { bytes })
                if bytes == MAX_DAEMON_FRAME_BYTES + 1
        ));
    }

    #[test]
    fn terminal_reader_ignores_unrelated_events_until_matching_terminal() {
        let envelope = shutdown_envelope();
        let other = shutdown_envelope();
        let unrelated = EventMsg::Completed {
            op_id: other.op_id,
            summary: Some("other op finished".to_string()),
            result: None,
        };
        let matching_progress = EventMsg::Progress {
            op_id: envelope.op_id.clone(),
            stage: "dispatch".to_string(),
            fraction: 0.5,
            message: None,
        };
        let matching_terminal = EventMsg::Completed {
            op_id: envelope.op_id.clone(),
            summary: Some("done".to_string()),
            result: None,
        };
        let trailing = EventMsg::Failed {
            op_id: envelope.op_id.clone(),
            error: "must not be read after terminal event".to_string(),
        };
        let mut bytes = Vec::new();
        for event in [
            unrelated,
            matching_progress.clone(),
            matching_terminal.clone(),
            trailing,
        ] {
            let json = serde_json::to_vec(&event).expect("serialize event");
            bytes.extend_from_slice(
                &u32::try_from(json.len())
                    .expect("event JSON length fits u32")
                    .to_be_bytes(),
            );
            bytes.extend_from_slice(&json);
        }

        let events = read_terminal_events(&mut Cursor::new(bytes), &envelope.op_id)
            .expect("matching terminal event should stop the reader");

        assert_eq!(events.len(), 3);
        assert_eq!(events[1], matching_progress);
        assert_eq!(events[2], matching_terminal);
    }

    #[test]
    fn http_sse_endpoint_derives_submission_path_from_events_path() {
        let root =
            parse_http_sse_endpoint("http://127.0.0.1:7879/events").expect("root events endpoint");
        assert_eq!(root.authority, "127.0.0.1:7879");
        assert_eq!(root.events_path, "/events");
        assert_eq!(root.submit_path, "/op");

        let nested = parse_http_sse_endpoint("http://127.0.0.1:7879/daemon/events")
            .expect("nested events endpoint");
        assert_eq!(nested.events_path, "/daemon/events");
        assert_eq!(nested.submit_path, "/daemon/op");
    }
}
