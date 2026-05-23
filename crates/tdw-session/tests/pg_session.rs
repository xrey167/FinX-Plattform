//! Integration test for the Postgres-backed session store —
//! full surface: sessions + permission rules + approvals + cost ledger.

#![cfg(feature = "postgres")]

use tdw_hooks::{PermissionEffect, PermissionRule, PermissionRules};
use tdw_protocol::{ApprovalDecision, PermissionId, SessionId};
use tdw_session::{CostLedgerEntry, PgSessionStore, SessionRecord, SessionStatus};
use tdw_storage_postgres::PgEngine;

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn base_table() -> String {
    format!("tdw_sessions_test_{}", std::process::id())
}

async fn drop_all(engine: &PgEngine, base: &str) {
    use tdw_core::RelationalEngine;
    for suffix in [
        "",
        "_permission_state",
        "_pending_approvals",
        "_cost_ledger",
    ] {
        let sql = format!("DROP TABLE IF EXISTS {base}{suffix}");
        engine
            .execute(&sql, serde_json::Value::Null)
            .await
            .unwrap_or_else(|error| panic!("drop {base}{suffix}: {error}"));
    }
}

#[tokio::test]
async fn pg_session_full_surface_roundtrip() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping pg_session integration test");
        return;
    };
    let engine = PgEngine::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres connect: {error}"));
    let base = base_table();
    let store = PgSessionStore::new(engine.clone()).with_table(&base);

    drop_all(&engine, &base).await;
    store
        .ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("ensure_schema: {error}"));

    // ─── session CRUD ─────────────────────────────────────────
    let session_id = SessionId::new("session-1").expect("session id");
    store
        .upsert_session(&SessionRecord {
            session_id: session_id.as_str().to_string(),
            status: SessionStatus::Active,
            created_at: "2026-05-23T00:00:00Z".to_string(),
            updated_at: "2026-05-23T00:00:00Z".to_string(),
        })
        .await
        .unwrap_or_else(|error| panic!("upsert: {error}"));

    let loaded = store
        .get_session(&session_id)
        .await
        .unwrap_or_else(|error| panic!("get_session: {error}"))
        .expect("session must exist");
    assert_eq!(loaded.status, SessionStatus::Active);

    // ─── permission rules ────────────────────────────────────
    let mut rules = PermissionRules::default();
    rules.push(PermissionRule::new(
        PermissionEffect::Allow,
        "tdw.query.*",
        "tdw.query.run",
    ));
    store
        .save_permission_rules(&session_id, &rules)
        .await
        .unwrap_or_else(|error| panic!("save_permission_rules: {error}"));
    let loaded_rules = store
        .load_permission_rules(&session_id)
        .await
        .unwrap_or_else(|error| panic!("load_permission_rules: {error}"))
        .expect("rules must exist");
    assert_eq!(
        loaded_rules.evaluate("tdw.query.run"),
        PermissionEffect::Allow
    );

    // ─── approvals ───────────────────────────────────────────
    let permission_id = PermissionId::new("permission-1").expect("permission id");
    store
        .request_approval(&session_id, &permission_id, "tdw.udf.run", "tdw.udf.*")
        .await
        .unwrap_or_else(|error| panic!("request_approval: {error}"));

    let pending = store
        .pending_approval(&permission_id)
        .await
        .unwrap_or_else(|error| panic!("pending_approval before resolve: {error}"))
        .expect("approval must exist");
    assert!(pending.decision.is_none());

    let resolved = store
        .resolve_approval(&permission_id, ApprovalDecision::AllowOnce)
        .await
        .unwrap_or_else(|error| panic!("resolve_approval: {error}"))
        .expect("approval must still exist after resolve");
    assert_eq!(resolved.decision, Some(ApprovalDecision::AllowOnce));

    // ─── cost ledger ─────────────────────────────────────────
    let cost = CostLedgerEntry {
        session_id: session_id.as_str().to_string(),
        operation_id: "op-1".to_string(),
        tokens: 42,
        bytes_scanned: 2048,
        rows_read: 128,
        rows_written: 4,
        backend: "postgres".to_string(),
    };
    store
        .append_cost(&cost)
        .await
        .unwrap_or_else(|error| panic!("append_cost: {error}"));

    let entries = store
        .cost_entries(&session_id)
        .await
        .unwrap_or_else(|error| panic!("cost_entries: {error}"));
    assert_eq!(entries, vec![cost]);

    drop_all(&engine, &base).await;
}
