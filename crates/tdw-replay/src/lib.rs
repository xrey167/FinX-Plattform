#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_cdc::CdcRecord;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayPlan {
    pub dry_run: bool,
    pub offsets: Vec<u64>,
    pub event_ids: Vec<String>,
}

pub struct ReplayEngine;

impl ReplayEngine {
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
}
