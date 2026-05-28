#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use ulid::Ulid;
use uuid::Uuid;

pub type Result<T> = std::result::Result<T, ProtocolError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        non_empty(value.into(), "session_id").map(Self)
    }

    #[must_use]
    pub fn generated() -> Self {
        Self(format!("session-{}", Ulid::new()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct OpId(String);

impl OpId {
    #[must_use]
    pub fn generated() -> Self {
        Self(Uuid::now_v7().to_string())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct PlanId(String);

impl PlanId {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        non_empty(value.into(), "plan_id").map(Self)
    }
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct PermissionId(String);

impl PermissionId {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        non_empty(value.into(), "permission_id").map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(
    Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ToolCallId(String);

impl ToolCallId {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        non_empty(value.into(), "tool_call_id").map(Self)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ActorKind {
    User,
    Service,
    Worker,
    Agent,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActorRef {
    pub actor_id: String,
    pub kind: ActorKind,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpEnvelope {
    pub op_id: OpId,
    pub session_id: SessionId,
    pub sequence: u64,
    pub submitted_by: ActorRef,
    pub op: Op,
}

impl OpEnvelope {
    #[must_use]
    pub fn new(session_id: SessionId, sequence: u64, submitted_by: ActorRef, op: Op) -> Self {
        Self {
            op_id: OpId::generated(),
            session_id,
            sequence,
            submitted_by,
            op,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TimeRange {
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CostHint {
    pub backend: String,
    pub estimated_bytes_scanned: Option<u64>,
    pub estimated_rows_read: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ApprovalDecision {
    AllowOnce,
    AlwaysAllow,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Op {
    AppendUserMessage {
        message: String,
    },
    RunQuery {
        sql: String,
        plan_id: Option<PlanId>,
        cost_hint: Option<CostHint>,
    },
    IngestBatch {
        provider: String,
        endpoint: String,
        range: Option<TimeRange>,
    },
    ToolCall {
        call_id: ToolCallId,
        tool_name: String,
        arguments: Value,
        permission_id: Option<PermissionId>,
    },
    ApprovalResponse {
        permission_id: PermissionId,
        decision: ApprovalDecision,
        reason: Option<String>,
    },
    CompactContext {
        target_tokens: u32,
    },
    Cancel {
        op_id: OpId,
    },
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventMsg {
    Started {
        op_id: OpId,
    },
    Progress {
        op_id: OpId,
        stage: String,
        fraction: f32,
        message: Option<String>,
    },
    ApprovalRequested {
        permission_id: PermissionId,
        action: String,
        pattern: String,
    },
    ToolCallRequested {
        op_id: OpId,
        call_id: ToolCallId,
        tool_name: String,
        arguments: Value,
    },
    ToolCallCompleted {
        op_id: OpId,
        call_id: ToolCallId,
        result: Value,
    },
    OutputChunk {
        op_id: OpId,
        stream: OutputStream,
        bytes: String,
    },
    DomainEvent {
        op_id: OpId,
        event_type: String,
        payload: Value,
    },
    Completed {
        op_id: OpId,
        summary: Option<String>,
        result: Option<Value>,
    },
    Failed {
        op_id: OpId,
        error: String,
    },
    Cancelled {
        op_id: OpId,
        reason: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum OutputStream {
    Stdout,
    Stderr,
    Model,
    System,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReplayFrame {
    pub session_id: SessionId,
    pub sequence: u64,
    pub event: EventMsg,
}

#[must_use]
pub fn schema_bundle() -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        ("op", schema_json::<Op>()),
        ("event_msg", schema_json::<EventMsg>()),
        ("op_envelope", schema_json::<OpEnvelope>()),
        ("replay_frame", schema_json::<ReplayFrame>()),
    ])
}

fn schema_json<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T))
        .unwrap_or_else(|error| panic!("protocol schema should serialize: {error}"))
}

fn non_empty(value: String, field: &'static str) -> Result<String> {
    if value.trim().is_empty() {
        Err(ProtocolError::EmptyField { field })
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn op_round_trips_with_tagged_variant() {
        let op = Op::RunQuery {
            sql: "select * from raw.market_data_bar".to_string(),
            plan_id: Some(PlanId::new("plan-1").expect("plan id")),
            cost_hint: Some(CostHint {
                backend: "clickhouse".to_string(),
                estimated_bytes_scanned: Some(2048),
                estimated_rows_read: Some(100),
            }),
        };

        let encoded = serde_json::to_value(&op).expect("op serializes");
        assert_eq!(encoded["type"], "run_query");

        let decoded: Op = serde_json::from_value(encoded).expect("op deserializes");
        assert_eq!(decoded, op);
    }

    #[test]
    fn rejects_empty_identifiers() {
        assert_eq!(
            SessionId::new(" "),
            Err(ProtocolError::EmptyField {
                field: "session_id"
            })
        );
        assert!(PermissionId::new("approval-1").is_ok());
    }

    #[test]
    fn event_messages_are_replayable() {
        let op_id = OpId::generated();
        let frame = ReplayFrame {
            session_id: SessionId::new("session-1").expect("session id"),
            sequence: 7,
            event: EventMsg::DomainEvent {
                op_id,
                event_type: "query.planned".to_string(),
                payload: json!({ "backend": "postgres" }),
            },
        };

        let encoded = serde_json::to_string(&frame).expect("frame serializes");
        let decoded: ReplayFrame = serde_json::from_str(&encoded).expect("frame deserializes");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn exports_protocol_schema_bundle() {
        let bundle = schema_bundle();
        for name in ["op", "event_msg", "op_envelope", "replay_frame"] {
            assert!(bundle.contains_key(name), "missing schema: {name}");
        }
    }
}
