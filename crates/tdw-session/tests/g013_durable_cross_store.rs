//! Cross-store G013 durable persistence smoke.
//!
//! Gated two ways:
//!   - Compiled only with `--features g013-cross-store`.
//!   - Runs only when `TDW_POSTGRES_TEST_URL` is set.

#![cfg(feature = "g013-cross-store")]

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use tdw_bus::PgEventBus;
use tdw_core::RelationalEngine;
use tdw_event::sample_event;
use tdw_outbox::PgOutboxStore;
use tdw_protocol::{EventMsg, OpId, ReplayFrame, SessionId};
use tdw_rollout::{JsonlRollout, RolloutRecord};
use tdw_session::{PgSessionStore, SessionRecord, SessionStatus};
use tdw_snapshot::PgSnapshotStore;
use tdw_storage_postgres::PgEngine;

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("{}_{}", std::process::id(), nanos)
}

async fn drop_table(engine: &PgEngine, table: &str) {
    let sql = format!("DROP TABLE IF EXISTS {table}");
    engine
        .execute(&sql, serde_json::Value::Null)
        .await
        .unwrap_or_else(|error| panic!("drop {table}: {error}"));
}

async fn drop_session_tables(engine: &PgEngine, base: &str) {
    for suffix in [
        "",
        "_permission_state",
        "_pending_approvals",
        "_cost_ledger",
    ] {
        drop_table(engine, &format!("{base}{suffix}")).await;
    }
}

#[tokio::test]
async fn durable_backends_share_postgres_and_rollout_archive() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping G013 cross-store test");
        return;
    };
    let suffix = suffix();
    let outbox_table = format!("tdw_g013_outbox_{suffix}");
    let bus_table = format!("tdw_g013_bus_{suffix}");
    let snapshot_table = format!("tdw_g013_snapshot_{suffix}");
    let session_base = format!("tdw_g013_sessions_{suffix}");
    let rollout_path = std::env::temp_dir().join(format!("tdw-g013-rollout-{suffix}.jsonl"));

    let engine = PgEngine::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres connect: {error}"));
    drop_table(&engine, &outbox_table).await;
    drop_table(&engine, &bus_table).await;
    drop_table(&engine, &snapshot_table).await;
    drop_session_tables(&engine, &session_base).await;
    let _ = fs::remove_file(&rollout_path);

    let outbox = PgOutboxStore::new(engine.clone())
        .with_table(&outbox_table)
        .expect("valid outbox table identifier");
    let bus = PgEventBus::new(engine.clone())
        .with_table(&bus_table)
        .expect("valid bus table identifier");
    let snapshot = PgSnapshotStore::new(engine.clone())
        .with_table(&snapshot_table)
        .expect("valid snapshot table identifier");
    let session = PgSessionStore::new(engine.clone())
        .with_table(&session_base)
        .expect("valid session table identifier");
    let rollout = JsonlRollout::new(&rollout_path);

    outbox
        .ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("outbox schema: {error}"));
    bus.ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("bus schema: {error}"));
    snapshot
        .ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("snapshot schema: {error}"));
    session
        .ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("session schema: {error}"));

    let event = sample_event("g013-cross-store");
    let outbox_sequence = outbox
        .append(event.clone())
        .await
        .unwrap_or_else(|error| panic!("outbox append: {error}"));
    let bus_sequence = bus
        .publish(event)
        .await
        .unwrap_or_else(|error| panic!("bus publish: {error}"));

    let session_id = SessionId::new("session-g013").expect("session id");
    session
        .upsert_session(&SessionRecord {
            session_id: session_id.as_str().to_string(),
            status: SessionStatus::Active,
            created_at: "2026-05-27T00:00:00Z".to_string(),
            updated_at: "2026-05-27T00:00:00Z".to_string(),
        })
        .await
        .unwrap_or_else(|error| panic!("session upsert: {error}"));

    let snapshot_record = snapshot
        .commit(
            "g013_cross_store",
            "2026-05-27T00:00:00Z",
            vec![
                format!("outbox:{outbox_sequence}"),
                format!("bus:{bus_sequence}"),
            ],
        )
        .await
        .unwrap_or_else(|error| panic!("snapshot commit: {error}"));

    rollout
        .append(&RolloutRecord {
            recorded_at: "2026-05-27T00:00:00Z".to_string(),
            frame: ReplayFrame {
                session_id: session_id.clone(),
                sequence: bus_sequence,
                event: EventMsg::Started {
                    op_id: OpId::generated(),
                },
            },
        })
        .unwrap_or_else(|error| panic!("rollout append: {error}"));

    let pending = outbox
        .pending_after(0)
        .await
        .unwrap_or_else(|error| panic!("outbox pending: {error}"));
    let bus_entries = bus
        .read_from(bus_sequence)
        .await
        .unwrap_or_else(|error| panic!("bus read: {error}"));
    let loaded_session = session
        .get_session(&session_id)
        .await
        .unwrap_or_else(|error| panic!("session load: {error}"))
        .expect("session must persist");
    let latest_snapshot = snapshot
        .latest("g013_cross_store")
        .await
        .unwrap_or_else(|error| panic!("snapshot latest: {error}"))
        .expect("snapshot must persist");
    let rollout_records = rollout
        .read_all()
        .unwrap_or_else(|error| panic!("rollout read: {error}"));

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].sequence, outbox_sequence);
    assert_eq!(bus_entries.len(), 1);
    assert_eq!(bus_entries[0].sequence, bus_sequence);
    assert_eq!(loaded_session.status, SessionStatus::Active);
    assert_eq!(snapshot_record.version, 1);
    assert_eq!(latest_snapshot.row_ids, snapshot_record.row_ids);
    assert_eq!(rollout_records.len(), 1);
    assert_eq!(rollout_records[0].frame.sequence, bus_sequence);

    drop_table(&engine, &outbox_table).await;
    drop_table(&engine, &bus_table).await;
    drop_table(&engine, &snapshot_table).await;
    drop_session_tables(&engine, &session_base).await;
    let _ = fs::remove_file(rollout_path);
}
