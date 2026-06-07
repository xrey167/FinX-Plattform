//! Offline `tdw-replay` example: build a protocol replay plan from rollout
//! records (input -> plan), without performing any replay.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-replay --example tdw_replay_basic
//! ```

#![forbid(unsafe_code)]

use tdw_protocol::{EventMsg, OpId, ReplayFrame, SessionId};
use tdw_replay::ReplayEngine;
use tdw_rollout::RolloutRecord;

fn main() {
    // A single recorded rollout frame carrying a `Completed` protocol event.
    let records = vec![RolloutRecord {
        recorded_at: "2026-05-22T00:00:00Z".to_string(),
        frame: ReplayFrame {
            session_id: SessionId::new("session-1").expect("session id"),
            sequence: 1,
            event: EventMsg::Completed {
                op_id: OpId::generated(),
                summary: Some("done".to_string()),
                result: None,
            },
        },
    }];

    // Planning maps each frame's sequence and event-type name. No replay runs.
    let plan = ReplayEngine::from_rollout(&records);
    assert_eq!(plan.sequences, vec![1]);
    assert_eq!(plan.event_types, vec!["completed".to_string()]);

    println!(
        "protocol replay plan: sequences={:?}, event_types={:?}",
        plan.sequences, plan.event_types
    );
}
