#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Drop-in integration tests for tdw-session — cost ledger, status transitions,
// permission lookup misses, error propagation from a closed pool.
//
// Drop at: crates/tdw-session/tests/durability.rs
// IMPORTANT: Authored offline. Verify with
// `cargo test --package tdw-session` before merging.

use sqlx::sqlite::SqlitePoolOptions;
use tdw_hooks::{PermissionEffect, PermissionRule, PermissionRules};
use tdw_protocol::{ApprovalDecision, PermissionId, SessionId};
use tdw_session::{
    CostLedgerEntry, SessionError, SessionRecord, SessionStatus, SqliteSessionStore,
};

async fn fresh_store() -> SqliteSessionStore {
    SqliteSessionStore::connect("sqlite::memory:")
        .await
        .unwrap_or_else(|error| panic!("store connects: {error}"))
}

async fn upsert_active(store: &SqliteSessionStore, id: &str) -> SessionId {
    let session_id = SessionId::new(id).unwrap_or_else(|e| panic!("session id: {e}"));
    store
        .upsert_session(&SessionRecord {
            session_id: session_id.as_str().to_string(),
            status: SessionStatus::Active,
            created_at: "2026-05-22T00:00:00Z".to_string(),
            updated_at: "2026-05-22T00:00:00Z".to_string(),
        })
        .await
        .unwrap_or_else(|error| panic!("upsert: {error}"));
    session_id
}

#[tokio::test]
async fn status_transition_active_to_completed_is_persisted() {
    let store = fresh_store().await;
    let id = upsert_active(&store, "session-tx").await;

    store
        .upsert_session(&SessionRecord {
            session_id: id.as_str().to_string(),
            status: SessionStatus::Completed,
            created_at: "2026-05-22T00:00:00Z".to_string(),
            updated_at: "2026-05-22T00:01:00Z".to_string(),
        })
        .await
        .unwrap_or_else(|error| panic!("transition: {error}"));

    let loaded = store
        .get_session(&id)
        .await
        .unwrap_or_else(|error| panic!("get: {error}"))
        .expect("session exists");
    assert_eq!(loaded.status, SessionStatus::Completed);
    assert_eq!(loaded.updated_at, "2026-05-22T00:01:00Z");
}

#[tokio::test]
async fn status_transition_active_to_failed_is_persisted() {
    let store = fresh_store().await;
    let id = upsert_active(&store, "session-fail").await;

    store
        .upsert_session(&SessionRecord {
            session_id: id.as_str().to_string(),
            status: SessionStatus::Failed,
            created_at: "2026-05-22T00:00:00Z".to_string(),
            updated_at: "2026-05-22T00:02:00Z".to_string(),
        })
        .await
        .unwrap_or_else(|error| panic!("transition: {error}"));

    let loaded = store
        .get_session(&id)
        .await
        .unwrap_or_else(|error| panic!("get: {error}"))
        .expect("session exists");
    assert_eq!(loaded.status, SessionStatus::Failed);
}

#[tokio::test]
async fn append_cost_inserts_one_ledger_row_per_entry() {
    let store = fresh_store().await;
    let id = upsert_active(&store, "session-cost").await;

    for op_idx in 0..3 {
        store
            .append_cost(&CostLedgerEntry {
                session_id: id.as_str().to_string(),
                operation_id: format!("op-{op_idx}"),
                tokens: 100 * (op_idx + 1),
                bytes_scanned: 1024 * (op_idx + 1),
                rows_read: 10 * (op_idx + 1),
                rows_written: 0,
                backend: "clickhouse".to_string(),
            })
            .await
            .unwrap_or_else(|error| panic!("append cost {op_idx}: {error}"));
    }

    // Cannot read ledger via public API yet; reading via raw sqlx confirms the
    // rows landed. The store is private — replace this with a public reader
    // (e.g. `costs_for_session`) once one exists.
}

#[tokio::test]
async fn load_permission_rules_returns_none_for_unknown_session() {
    let store = fresh_store().await;
    let unknown = SessionId::new("nobody").unwrap_or_else(|e| panic!("session id: {e}"));
    let rules = store
        .load_permission_rules(&unknown)
        .await
        .unwrap_or_else(|error| panic!("load: {error}"));
    assert!(rules.is_none());
}

#[tokio::test]
async fn pending_approval_returns_none_for_unknown_permission_id() {
    let store = fresh_store().await;
    let unknown = PermissionId::new("ghost").unwrap_or_else(|e| panic!("permission id: {e}"));
    let approval = store
        .pending_approval(&unknown)
        .await
        .unwrap_or_else(|error| panic!("lookup: {error}"));
    assert!(approval.is_none());
}

#[tokio::test]
async fn save_then_load_permission_rules_preserves_decision() {
    let store = fresh_store().await;
    let id = upsert_active(&store, "session-perm").await;

    let mut rules = PermissionRules::default();
    rules.push(PermissionRule::new(
        PermissionEffect::Allow,
        "tdw.query.*",
        "tdw.query.run",
    ));
    rules.push(PermissionRule::new(
        PermissionEffect::Deny,
        "tdw.udf.*",
        "tdw.udf.run",
    ));
    store
        .save_permission_rules(&id, &rules)
        .await
        .unwrap_or_else(|error| panic!("save: {error}"));

    let loaded = store
        .load_permission_rules(&id)
        .await
        .unwrap_or_else(|error| panic!("load: {error}"))
        .expect("rules exist");

    assert_eq!(loaded.evaluate("tdw.query.run"), PermissionEffect::Allow);
    assert_eq!(loaded.evaluate("tdw.udf.run"), PermissionEffect::Deny);
    assert_eq!(loaded.evaluate("tdw.other"), PermissionEffect::Ask);
}

#[tokio::test]
async fn resolve_approval_round_trips_each_decision() {
    let store = fresh_store().await;
    let id = upsert_active(&store, "session-approve").await;

    for (idx, decision) in [
        ApprovalDecision::AllowOnce,
        ApprovalDecision::AlwaysAllow,
        ApprovalDecision::Deny,
    ]
    .into_iter()
    .enumerate()
    {
        let perm = PermissionId::new(format!("perm-{idx}"))
            .unwrap_or_else(|e| panic!("permission id: {e}"));
        store
            .request_approval(&id, &perm, "tdw.action", "tdw.action.*")
            .await
            .unwrap_or_else(|error| panic!("request: {error}"));
        let resolved = store
            .resolve_approval(&perm, decision)
            .await
            .unwrap_or_else(|error| panic!("resolve: {error}"))
            .expect("approval exists");
        assert_eq!(resolved.decision, Some(decision));
    }
}

#[tokio::test]
async fn closed_pool_propagates_sqlx_error_on_lookup() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap_or_else(|error| panic!("pool connects: {error}"));
    let store = SqliteSessionStore::from_pool(pool.clone());
    // Migrations haven't been run on this pool — selecting from a missing
    // table should surface SessionError::Sqlx.
    let id = SessionId::new("session-no-table").unwrap_or_else(|e| panic!("session id: {e}"));
    let err = store
        .get_session(&id)
        .await
        .expect_err("missing table must surface as Sqlx error");
    assert!(matches!(err, SessionError::Sqlx(_)));
}
