//! Cross-backend behavioral conformance for the session store (go-live P2.6,
//! slice 3 + G007 SE1/SE2 close-out).
//!
//! ONE suite of assertions runs verbatim against the `SQLite` store and the
//! Postgres store:
//!
//! - **SE3** `append_cost` is idempotent per `(session_id, operation_id)` —
//!   the at-least-once op pipeline must never double-count a retried append.
//! - **SE1** `resolve_approval` is first-decision-wins — a resolved approval
//!   cannot be flipped by a second caller.
//! - **SE2** `upsert_session` cannot reset a terminal (`Completed`/`Failed`)
//!   session back to `Active`.
//!
//! The Postgres leg compiles with `--features postgres` and self-skips unless
//! `TDW_POSTGRES_TEST_URL` is set, mirroring `tests/pg_session.rs`.

#![forbid(unsafe_code)]

use tdw_protocol::{ApprovalDecision, PermissionId, SessionId};
use tdw_session::{
    CostLedgerEntry, PendingApprovalRecord, SessionRecord, SessionStatus, SqliteSessionStore,
};

enum AnyStore {
    Sqlite(SqliteSessionStore),
    #[cfg(feature = "postgres")]
    Pg(tdw_session::PgSessionStore),
}

impl AnyStore {
    async fn upsert_session(&self, record: &SessionRecord) {
        match self {
            Self::Sqlite(store) => store
                .upsert_session(record)
                .await
                .unwrap_or_else(|error| panic!("upsert_session: {error}")),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .upsert_session(record)
                .await
                .unwrap_or_else(|error| panic!("upsert_session: {error}")),
        }
    }

    async fn get_session(&self, session_id: &SessionId) -> Option<SessionRecord> {
        match self {
            Self::Sqlite(store) => store
                .get_session(session_id)
                .await
                .unwrap_or_else(|error| panic!("get_session: {error}")),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .get_session(session_id)
                .await
                .unwrap_or_else(|error| panic!("get_session: {error}")),
        }
    }

    async fn request_approval(
        &self,
        session_id: &SessionId,
        permission_id: &PermissionId,
        tool: &str,
        pattern: &str,
    ) {
        match self {
            Self::Sqlite(store) => store
                .request_approval(session_id, permission_id, tool, pattern)
                .await
                .unwrap_or_else(|error| panic!("request_approval: {error}")),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .request_approval(session_id, permission_id, tool, pattern)
                .await
                .unwrap_or_else(|error| panic!("request_approval: {error}")),
        }
    }

    async fn resolve_approval(
        &self,
        permission_id: &PermissionId,
        decision: ApprovalDecision,
    ) -> Option<PendingApprovalRecord> {
        match self {
            Self::Sqlite(store) => store
                .resolve_approval(permission_id, decision)
                .await
                .unwrap_or_else(|error| panic!("resolve_approval: {error}")),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .resolve_approval(permission_id, decision)
                .await
                .unwrap_or_else(|error| panic!("resolve_approval: {error}")),
        }
    }

    async fn append_cost(&self, entry: &CostLedgerEntry) {
        match self {
            Self::Sqlite(store) => store
                .append_cost(entry)
                .await
                .unwrap_or_else(|error| panic!("append_cost must succeed: {error}")),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .append_cost(entry)
                .await
                .unwrap_or_else(|error| panic!("append_cost must succeed: {error}")),
        }
    }

    async fn cost_entries(&self, session_id: &SessionId) -> Vec<CostLedgerEntry> {
        match self {
            Self::Sqlite(store) => store
                .cost_entries(session_id)
                .await
                .unwrap_or_else(|error| panic!("cost_entries: {error}")),
            #[cfg(feature = "postgres")]
            Self::Pg(store) => store
                .cost_entries(session_id)
                .await
                .unwrap_or_else(|error| panic!("cost_entries: {error}")),
        }
    }
}

fn active_record(session_id: &str) -> SessionRecord {
    SessionRecord {
        session_id: session_id.to_string(),
        status: SessionStatus::Active,
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:00:00Z".to_string(),
    }
}

fn terminal_record(session_id: &str, status: SessionStatus) -> SessionRecord {
    SessionRecord {
        session_id: session_id.to_string(),
        status,
        created_at: "2026-06-10T00:00:00Z".to_string(),
        updated_at: "2026-06-10T00:01:00Z".to_string(),
    }
}

fn cost(session_id: &str, operation_id: &str, tokens: i64) -> CostLedgerEntry {
    CostLedgerEntry {
        session_id: session_id.to_string(),
        operation_id: operation_id.to_string(),
        tokens,
        bytes_scanned: 100,
        rows_read: 10,
        rows_written: 1,
        backend: "conformance".to_string(),
    }
}

/// The full behavioral contract, asserted identically for every backend.
async fn conformance_suite(store: &AnyStore, prefix: &str) {
    // ── SE3: append_cost idempotency ─────────────────────────────────────────
    {
        let session_a = format!("{prefix}se3a");
        let session_b = format!("{prefix}se3b");
        let sid_a = SessionId::new(&session_a).expect("session id a");
        let sid_b = SessionId::new(&session_b).expect("session id b");
        store.upsert_session(&active_record(&session_a)).await;
        store.upsert_session(&active_record(&session_b)).await;

        // Idempotent append per (session_id, operation_id): the at-least-once
        // op pipeline may retry — a duplicate must be a no-op, not an error
        // and not a double-count.
        let entry = cost(&session_a, "op-1", 42);
        store.append_cost(&entry).await;
        store.append_cost(&entry).await;
        let after_dup = store.cost_entries(&sid_a).await;
        assert_eq!(
            after_dup.len(),
            1,
            "SE3: duplicate append must not double-count"
        );
        assert_eq!(after_dup[0].tokens, 42);
        assert_eq!(after_dup[0].operation_id, "op-1");

        // Distinct operations accumulate.
        store.append_cost(&cost(&session_a, "op-2", 7)).await;
        let entries = store.cost_entries(&sid_a).await;
        assert_eq!(entries.len(), 2, "SE3: distinct operations accumulate");
        let mut ops: Vec<&str> = entries
            .iter()
            .map(|entry| entry.operation_id.as_str())
            .collect();
        ops.sort_unstable();
        assert_eq!(ops, vec!["op-1", "op-2"]);

        // Session scoping: another session's ledger is untouched.
        store.append_cost(&cost(&session_b, "op-1", 5)).await;
        let b_entries = store.cost_entries(&sid_b).await;
        assert_eq!(b_entries.len(), 1, "SE3: sessions are isolated");
        assert_eq!(b_entries[0].tokens, 5);
        assert_eq!(
            store.cost_entries(&sid_a).await.len(),
            2,
            "SE3: appending to b must not leak into a"
        );
    }

    // ── SE1: resolve_approval is first-decision-wins ─────────────────────────
    {
        let session = format!("{prefix}se1");
        let sid = SessionId::new(&session).expect("se1 session id");
        let perm_id = format!("{prefix}se1-perm");
        let perm = PermissionId::new(&perm_id).expect("se1 permission id");

        store.upsert_session(&active_record(&session)).await;
        store
            .request_approval(&sid, &perm, "tdw.udf.run", "tdw.udf.*")
            .await;

        // First decision is recorded and returned.
        let first = store
            .resolve_approval(&perm, ApprovalDecision::AllowOnce)
            .await
            .expect("approval exists after first resolve");
        assert_eq!(
            first.decision,
            Some(ApprovalDecision::AllowOnce),
            "SE1: first decision must be recorded"
        );

        // A second resolve must NOT overwrite the already-decided approval:
        // the `decision IS NULL` guard makes the update a no-op.
        let second = store
            .resolve_approval(&perm, ApprovalDecision::Deny)
            .await
            .expect("approval exists after second resolve");
        assert_eq!(
            second.decision,
            Some(ApprovalDecision::AllowOnce),
            "SE1: second resolve must not flip an already-decided approval"
        );
    }

    // ── SE2: terminal sessions cannot be reset to Active ─────────────────────
    for terminal_status in [SessionStatus::Completed, SessionStatus::Failed] {
        let label = format!("{terminal_status:?}").to_lowercase();
        let session = format!("{prefix}se2-{label}");
        let sid = SessionId::new(&session).expect("se2 session id");

        // Transition Active -> terminal.
        store.upsert_session(&active_record(&session)).await;
        store
            .upsert_session(&terminal_record(&session, terminal_status))
            .await;

        let after_terminal = store
            .get_session(&sid)
            .await
            .expect("session must exist after terminal upsert");
        assert_eq!(
            after_terminal.status, terminal_status,
            "SE2: upsert to {terminal_status:?} must persist"
        );

        // Attempt to reset back to Active — must be silently ignored.
        store.upsert_session(&active_record(&session)).await;

        let after_reset_attempt = store
            .get_session(&sid)
            .await
            .expect("session must still exist after reset attempt");
        assert_eq!(
            after_reset_attempt.status, terminal_status,
            "SE2: a terminal ({terminal_status:?}) session must not be reset to Active"
        );
    }
}

#[tokio::test]
async fn sqlite_backend_conforms() {
    let store = SqliteSessionStore::connect("sqlite::memory:")
        .await
        .unwrap_or_else(|error| panic!("sqlite connect: {error}"));
    conformance_suite(&AnyStore::Sqlite(store), "conf_sqlite_").await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_conforms() {
    use tdw_core::RelationalEngine;
    use tdw_storage_postgres::PgEngine;

    let Some(url) = std::env::var("TDW_POSTGRES_TEST_URL").ok() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping postgres conformance leg");
        return;
    };
    let engine = PgEngine::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres connect: {error}"));

    // Hermetic per-run base table, mirroring tests/pg_session.rs.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let base = format!("tdw_sessions_conf_{}_{}", std::process::id(), nanos);
    let store = tdw_session::PgSessionStore::new(engine.clone())
        .with_table(&base)
        .expect("valid session table identifier");
    store
        .ensure_schema()
        .await
        .unwrap_or_else(|error| panic!("ensure_schema: {error}"));

    conformance_suite(&AnyStore::Pg(store), "conf_pg_").await;

    for suffix in [
        "_cost_ledger",
        "_pending_approvals",
        "_permission_state",
        "",
    ] {
        let drop_sql = format!("DROP TABLE IF EXISTS {base}{suffix}");
        engine
            .execute(&drop_sql, serde_json::Value::Null)
            .await
            .unwrap_or_else(|error| panic!("cleanup drop: {error}"));
    }
}
