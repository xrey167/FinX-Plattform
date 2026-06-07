//! Offline, no-network example for the `tdw-cli` client crate.
//!
//! Shows the two `OpEnvelope`s the CLI builds (`run-query` and the default
//! `Shutdown`) and frames them to JSON exactly as the CLI writes them on the
//! wire, then runs the offline `--smoke` end-to-end check. No daemon is
//! contacted — no socket, no network.
//!
//! Run with: `cargo run -p tdw-cli --example tdw_cli_basic`

use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};
use tdw_test_utils::smoke::{allocate_storage_root, run_end_to_end_smoke};

fn cli_envelope(op: Op) -> OpEnvelope {
    OpEnvelope::new(
        SessionId::generated(),
        1,
        ActorRef {
            actor_id: "user:tdw-cli".to_string(),
            kind: ActorKind::User,
            tenant_id: None,
        },
        op,
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. The envelope `tdw-cli run-query "select 1"` would frame.
    let run_query = cli_envelope(Op::RunQuery {
        sql: "select 1".to_string(),
        plan_id: None,
        cost_hint: None,
    });
    println!("run-query envelope: {}", serde_json::to_string(&run_query)?);

    // 2. The envelope the default (no-arg) `tdw-cli` invocation would frame.
    let shutdown = cli_envelope(Op::Shutdown);
    println!("default envelope:   {}", serde_json::to_string(&shutdown)?);

    // 3. The offline `--smoke` end-to-end check the CLI also exposes.
    let root = allocate_storage_root("tdw-cli-example");
    let report = run_end_to_end_smoke("AAPL", root.clone()).await?;
    println!(
        "smoke: provider={} endpoint={} symbol={} rows={} roundtrip={}",
        report.provider,
        report.endpoint,
        report.query_symbol,
        report.rows_fetched,
        report.roundtrip_ok,
    );
    let _ = std::fs::remove_dir_all(&root);

    Ok(())
}
