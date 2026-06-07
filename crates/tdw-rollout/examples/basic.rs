//! Offline `JsonlRollout` round-trip: append two replay frames to a temp JSONL
//! file and read them back. No network, no docker — the local-fs JSONL backend
//! is the always-available default.
//!
//! Run with: `cargo run -p tdw-rollout --example tdw-rollout-basic`

use tdw_protocol::{EventMsg, OpId, ReplayFrame, SessionId};
use tdw_rollout::{JsonlRollout, RolloutRecord};

fn record(sequence: u64) -> RolloutRecord {
    RolloutRecord {
        recorded_at: "2026-05-22T00:00:00Z".to_string(),
        frame: ReplayFrame {
            session_id: SessionId::new("session-1").expect("session id"),
            sequence,
            event: EventMsg::Started {
                op_id: OpId::generated(),
            },
        },
    }
}

fn main() -> tdw_rollout::Result<()> {
    let path =
        std::env::temp_dir().join(format!("tdw-rollout-example-{}.jsonl", std::process::id()));
    let rollout = JsonlRollout::new(&path);

    // Each append is flock-serialized and fsynced before it returns.
    rollout.append(&record(1))?;
    rollout.append(&record(2))?;

    let all = rollout.read_all()?;
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].frame.sequence, 1);
    assert_eq!(all[1].frame.sequence, 2);

    println!(
        "rollout ok: wrote {} frames to {}, sequences = {:?}",
        all.len(),
        path.display(),
        all.iter().map(|r| r.frame.sequence).collect::<Vec<_>>()
    );

    let _ = std::fs::remove_file(&path);
    Ok(())
}
