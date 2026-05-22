#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_app_server::{DaemonEndpoint, SubmissionError, SubmissionHandle};
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

impl AppClient {
    pub fn new(info: ClientInfo, submissions: SubmissionHandle) -> Self {
        Self { info, submissions }
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
        let client = AppClient::new(
            ClientInfo {
                name: "tdw-cli".to_string(),
                endpoint: DaemonEndpoint {
                    transport: DaemonTransport::Uds,
                    address: "~/.tdw/daemon.sock".to_string(),
                },
            },
            handle,
        );
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
}
