#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_cdc::CdcRecord;
use tdw_rollout::RolloutRecord;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayPlan {
    pub dry_run: bool,
    pub offsets: Vec<u64>,
    pub event_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolReplayPlan {
    pub sequences: Vec<u64>,
    pub event_types: Vec<String>,
}

pub struct ReplayEngine;

impl ReplayEngine {
    #[must_use]
    pub fn dry_run(records: &[CdcRecord]) -> ReplayPlan {
        ReplayPlan {
            dry_run: true,
            offsets: records.iter().map(|record| record.offset).collect(),
            event_ids: records
                .iter()
                .map(|record| record.event_id.clone())
                .collect(),
        }
    }

    #[must_use]
    pub fn from_rollout(records: &[RolloutRecord]) -> ProtocolReplayPlan {
        ProtocolReplayPlan {
            sequences: records.iter().map(|record| record.frame.sequence).collect(),
            event_types: records
                .iter()
                .map(|record| event_type_name(&record.frame.event).to_string())
                .collect(),
        }
    }
}

const fn event_type_name(event: &tdw_protocol::EventMsg) -> &'static str {
    match event {
        tdw_protocol::EventMsg::Started { .. } => "started",
        tdw_protocol::EventMsg::Progress { .. } => "progress",
        tdw_protocol::EventMsg::ApprovalRequested { .. } => "approval_requested",
        tdw_protocol::EventMsg::ToolCallRequested { .. } => "tool_call_requested",
        tdw_protocol::EventMsg::ToolCallCompleted { .. } => "tool_call_completed",
        tdw_protocol::EventMsg::OutputChunk { .. } => "output_chunk",
        tdw_protocol::EventMsg::DomainEvent { .. } => "domain_event",
        tdw_protocol::EventMsg::Completed { .. } => "completed",
        tdw_protocol::EventMsg::Failed { .. } => "failed",
        tdw_protocol::EventMsg::Cancelled { .. } => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tdw_cdc::CdcRecord;

    #[test]
    fn dry_run_reports_replay_offsets_without_mutation() {
        let records = vec![CdcRecord {
            offset: 7,
            event_id: "evt-7".to_string(),
            event_type: "ingress.received".to_string(),
            payload: json!({"ok": true}),
        }];

        let plan = ReplayEngine::dry_run(&records);
        assert!(plan.dry_run);
        assert_eq!(plan.offsets, vec![7]);
        assert_eq!(plan.event_ids, vec!["evt-7"]);
    }

    #[test]
    fn protocol_replay_reads_rollout_event_types() {
        let records = vec![RolloutRecord {
            recorded_at: "2026-05-22T00:00:00Z".to_string(),
            frame: tdw_protocol::ReplayFrame {
                session_id: tdw_protocol::SessionId::new("session-1").expect("session id"),
                sequence: 1,
                event: tdw_protocol::EventMsg::Completed {
                    op_id: tdw_protocol::OpId::generated(),
                    summary: Some("done".to_string()),
                    result: None,
                },
            },
        }];

        let plan = ReplayEngine::from_rollout(&records);

        assert_eq!(plan.sequences, vec![1]);
        assert_eq!(plan.event_types, vec!["completed"]);
    }
}
