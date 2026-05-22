#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::json;
use tdw_protocol::{EventMsg, Op, OpEnvelope};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecRun {
    pub events: Vec<EventMsg>,
}

pub fn run_headless(envelope: OpEnvelope) -> ExecRun {
    let mut events = vec![EventMsg::Started {
        op_id: envelope.op_id.clone(),
    }];
    events.push(match envelope.op {
        Op::RunQuery { sql, .. } => EventMsg::Completed {
            op_id: envelope.op_id,
            summary: Some("query planned".to_string()),
            result: Some(json!({ "sql": sql })),
        },
        other => EventMsg::Completed {
            op_id: envelope.op_id,
            summary: Some("op accepted".to_string()),
            result: Some(json!({ "op": other })),
        },
    });
    ExecRun { events }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, SessionId};

    #[test]
    fn headless_exec_returns_protocol_events() {
        let envelope = OpEnvelope::new(
            SessionId::new("session-1").expect("session id"),
            1,
            ActorRef {
                actor_id: "user".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::RunQuery {
                sql: "select 1".to_string(),
                plan_id: None,
                cost_hint: None,
            },
        );
        let run = run_headless(envelope);

        assert!(matches!(run.events[0], EventMsg::Started { .. }));
        assert!(matches!(run.events[1], EventMsg::Completed { .. }));
    }
}
