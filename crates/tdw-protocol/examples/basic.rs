//! Offline, no-network example for `tdw-protocol`.
//!
//! Builds an `OpEnvelope`, round-trips it through JSON exactly as the daemon
//! transport does, constructs the matching terminal `EventMsg`, wraps it in a
//! `ReplayFrame`, and prints the exported schema names. No I/O beyond stdout.
//!
//! Run with: `cargo run -p tdw-protocol --example tdw_protocol_basic`

use tdw_protocol::{
    ActorKind, ActorRef, EventMsg, Op, OpEnvelope, ReplayFrame, SessionId, schema_bundle,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Build an op envelope, the unit a client submits to the daemon.
    let session = SessionId::new("session-basic")?;
    let envelope = OpEnvelope::new(
        session.clone(),
        1,
        ActorRef {
            actor_id: "user:example".to_string(),
            kind: ActorKind::User,
            tenant_id: Some("default".to_string()),
        },
        Op::RunQuery {
            sql: "select 1".to_string(),
            plan_id: None,
            cost_hint: None,
        },
    );

    // 2. Round-trip through JSON: serialize (what the client writes on the wire)
    //    then deserialize (what the daemon reads). Identity must hold.
    let encoded = serde_json::to_string(&envelope)?;
    let decoded: OpEnvelope = serde_json::from_str(&encoded)?;
    assert_eq!(decoded.op_id, envelope.op_id);
    assert_eq!(decoded.session_id, envelope.session_id);
    println!("op envelope round-trips: op_id={}", decoded.op_id.as_str());

    // 3. Construct the terminal event the daemon would emit for this op, then
    //    record it as a replay frame (what rollout/replay persist).
    let event = EventMsg::Completed {
        op_id: decoded.op_id,
        summary: Some("ok".to_string()),
        result: None,
    };
    let frame = ReplayFrame {
        session_id: session,
        sequence: 1,
        event,
    };
    let frame_json = serde_json::to_string(&frame)?;
    let frame_back: ReplayFrame = serde_json::from_str(&frame_json)?;
    assert_eq!(frame_back, frame);
    println!(
        "replay frame round-trips at sequence {}",
        frame_back.sequence
    );

    // 4. The protocol exports JSON Schemas for its wire types.
    let names: Vec<&str> = schema_bundle().keys().copied().collect();
    println!("exported schemas: {names:?}");

    Ok(())
}
