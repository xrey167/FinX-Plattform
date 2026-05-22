#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;
use tdw_app_server::{DaemonEndpoint, EndpointError, SubmissionError, SubmissionHandle};
use tdw_protocol::OpEnvelope;

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
}
