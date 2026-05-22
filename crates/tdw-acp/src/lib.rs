#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_protocol::{EventMsg, Op, SessionId};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpServerInfo {
    pub name: String,
    pub protocol_version: String,
    pub supports_streaming: bool,
    pub supports_approvals: bool,
}

impl Default for AcpServerInfo {
    fn default() -> Self {
        Self {
            name: "tdw-acp".to_string(),
            protocol_version: "0.1.0".to_string(),
            supports_streaming: true,
            supports_approvals: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpRequest {
    Initialize {
        client_name: String,
    },
    SubmitOp {
        session_id: SessionId,
        op: Op,
    },
    ResolveApproval {
        permission_id: String,
        decision: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcpResponse {
    Initialized {
        server: AcpServerInfo,
    },
    Event {
        session_id: SessionId,
        event: EventMsg,
    },
    Error {
        message: String,
        data: Option<Value>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, OpEnvelope};

    #[test]
    fn acp_request_serializes_protocol_op() {
        let request = AcpRequest::SubmitOp {
            session_id: SessionId::new("session-1").expect("session id"),
            op: Op::AppendUserMessage {
                message: "hello".to_string(),
            },
        };

        let encoded = serde_json::to_value(&request).expect("request serializes");
        assert_eq!(encoded["type"], "submit_op");
        assert_eq!(encoded["op"]["type"], "append_user_message");
    }

    #[test]
    fn acp_response_can_wrap_event_msg() {
        let session_id = SessionId::new("session-1").expect("session id");
        let envelope = OpEnvelope::new(
            session_id.clone(),
            1,
            ActorRef {
                actor_id: "client".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::Shutdown,
        );
        let response = AcpResponse::Event {
            session_id,
            event: EventMsg::Started {
                op_id: envelope.op_id,
            },
        };

        let encoded = serde_json::to_value(&response).expect("response serializes");
        assert_eq!(encoded["type"], "event");
        assert_eq!(encoded["event"]["type"], "started");
    }
}
