#![forbid(unsafe_code)]

#[cfg(feature = "postgres")]
pub mod pg_session;

#[cfg(feature = "postgres")]
pub use pg_session::PgSessionStore;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};
use tdw_hooks::PermissionRules;
use tdw_protocol::{ApprovalDecision, PermissionId, SessionId};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid session status: {0}")]
    InvalidSessionStatus(String),
    #[error("invalid approval decision: {0}")]
    InvalidApprovalDecision(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub status: SessionStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    Active,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingApprovalRecord {
    pub permission_id: String,
    pub session_id: String,
    pub action: String,
    pub pattern: String,
    pub decision: Option<ApprovalDecision>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostLedgerEntry {
    pub session_id: String,
    pub operation_id: String,
    pub tokens: i64,
    pub bytes_scanned: i64,
    pub rows_read: i64,
    pub rows_written: i64,
    pub backend: String,
}

#[derive(Clone)]
pub struct SqliteSessionStore {
    pool: SqlitePool,
}

impl SqliteSessionStore {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(database_url)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        for statement in SQLITE_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn upsert_session(&self, record: &SessionRecord) -> Result<()> {
        sqlx::query(
            r#"
            insert into sessions (session_id, status, created_at, updated_at)
            values (?1, ?2, ?3, ?4)
            on conflict(session_id) do update set
                status = excluded.status,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&record.session_id)
        .bind(format!("{:?}", record.status))
        .bind(&record.created_at)
        .bind(&record.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session(&self, session_id: &SessionId) -> Result<Option<SessionRecord>> {
        let row = sqlx::query(
            "select session_id, status, created_at, updated_at from sessions where session_id = ?1",
        )
        .bind(session_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_session).transpose()
    }

    pub async fn save_permission_rules(
        &self,
        session_id: &SessionId,
        rules: &PermissionRules,
    ) -> Result<()> {
        let rules_json = serde_json::to_string(rules)?;
        sqlx::query(
            r#"
            insert into permission_state (session_id, rules_json)
            values (?1, ?2)
            on conflict(session_id) do update set rules_json = excluded.rules_json
            "#,
        )
        .bind(session_id.as_str())
        .bind(rules_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_permission_rules(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<PermissionRules>> {
        let row = sqlx::query("select rules_json from permission_state where session_id = ?1")
            .bind(session_id.as_str())
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| serde_json::from_str(row.get::<&str, _>("rules_json")))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn request_approval(
        &self,
        session_id: &SessionId,
        permission_id: &PermissionId,
        action: &str,
        pattern: &str,
    ) -> Result<()> {
        sqlx::query(
            r#"
            insert into pending_approvals (permission_id, session_id, action, pattern, decision)
            values (?1, ?2, ?3, ?4, null)
            "#,
        )
        .bind(permission_id.as_str())
        .bind(session_id.as_str())
        .bind(action)
        .bind(pattern)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn resolve_approval(
        &self,
        permission_id: &PermissionId,
        decision: ApprovalDecision,
    ) -> Result<Option<PendingApprovalRecord>> {
        let decision_text = format!("{decision:?}");
        sqlx::query("update pending_approvals set decision = ?1 where permission_id = ?2")
            .bind(decision_text)
            .bind(permission_id.as_str())
            .execute(&self.pool)
            .await?;
        self.pending_approval(permission_id).await
    }

    pub async fn pending_approval(
        &self,
        permission_id: &PermissionId,
    ) -> Result<Option<PendingApprovalRecord>> {
        let row = sqlx::query(
            "select permission_id, session_id, action, pattern, decision from pending_approvals where permission_id = ?1",
        )
        .bind(permission_id.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_pending_approval).transpose()
    }

    pub async fn append_cost(&self, entry: &CostLedgerEntry) -> Result<()> {
        sqlx::query(
            r#"
            insert into cost_ledger
                (session_id, operation_id, tokens, bytes_scanned, rows_read, rows_written, backend)
            values (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(&entry.session_id)
        .bind(&entry.operation_id)
        .bind(entry.tokens)
        .bind(entry.bytes_scanned)
        .bind(entry.rows_read)
        .bind(entry.rows_written)
        .bind(&entry.backend)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn cost_entries(&self, session_id: &SessionId) -> Result<Vec<CostLedgerEntry>> {
        let rows = sqlx::query(
            r#"
            select session_id, operation_id, tokens, bytes_scanned, rows_read, rows_written, backend
            from cost_ledger
            where session_id = ?1
            order by id asc
            "#,
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_cost_entry).collect())
    }
}

pub const SQLITE_MIGRATION: &str = r#"
create table if not exists sessions (
    session_id text primary key,
    status text not null,
    created_at text not null,
    updated_at text not null
);
create table if not exists permission_state (
    session_id text primary key references sessions(session_id),
    rules_json text not null
);
create table if not exists pending_approvals (
    permission_id text primary key,
    session_id text not null references sessions(session_id),
    action text not null,
    pattern text not null,
    decision text
);
create table if not exists cost_ledger (
    id integer primary key autoincrement,
    session_id text not null references sessions(session_id),
    operation_id text not null,
    tokens integer not null,
    bytes_scanned integer not null,
    rows_read integer not null,
    rows_written integer not null,
    backend text not null
);
"#;

fn row_to_session(row: sqlx::sqlite::SqliteRow) -> Result<SessionRecord> {
    let status_text = row.get::<String, _>("status");
    let status = match status_text.as_str() {
        "Active" => SessionStatus::Active,
        "Completed" => SessionStatus::Completed,
        "Failed" => SessionStatus::Failed,
        other => return Err(SessionError::InvalidSessionStatus(other.to_string())),
    };
    Ok(SessionRecord {
        session_id: row.get("session_id"),
        status,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_pending_approval(row: sqlx::sqlite::SqliteRow) -> Result<PendingApprovalRecord> {
    let decision = match row.get::<Option<String>, _>("decision").as_deref() {
        None => None,
        Some("AllowOnce") => Some(ApprovalDecision::AllowOnce),
        Some("AlwaysAllow") => Some(ApprovalDecision::AlwaysAllow),
        Some("Deny") => Some(ApprovalDecision::Deny),
        Some(other) => return Err(SessionError::InvalidApprovalDecision(other.to_string())),
    };
    Ok(PendingApprovalRecord {
        permission_id: row.get("permission_id"),
        session_id: row.get("session_id"),
        action: row.get("action"),
        pattern: row.get("pattern"),
        decision,
    })
}

fn row_to_cost_entry(row: sqlx::sqlite::SqliteRow) -> CostLedgerEntry {
    CostLedgerEntry {
        session_id: row.get("session_id"),
        operation_id: row.get("operation_id"),
        tokens: row.get("tokens"),
        bytes_scanned: row.get("bytes_scanned"),
        rows_read: row.get("rows_read"),
        rows_written: row.get("rows_written"),
        backend: row.get("backend"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_hooks::{PermissionEffect, PermissionRule};

    #[tokio::test]
    async fn sqlite_store_persists_session_permissions_and_approvals() {
        let store = SqliteSessionStore::connect("sqlite::memory:")
            .await
            .unwrap_or_else(|error| panic!("store connects: {error}"));
        let session_id = SessionId::new("session-1").expect("session id");
        store
            .upsert_session(&SessionRecord {
                session_id: session_id.as_str().to_string(),
                status: SessionStatus::Active,
                created_at: "2026-05-22T00:00:00Z".to_string(),
                updated_at: "2026-05-22T00:00:00Z".to_string(),
            })
            .await
            .unwrap_or_else(|error| panic!("session persists: {error}"));

        let mut rules = PermissionRules::default();
        rules.push(PermissionRule::new(
            PermissionEffect::Allow,
            "tdw.query.*",
            "tdw.query.run",
        ));
        store
            .save_permission_rules(&session_id, &rules)
            .await
            .unwrap_or_else(|error| panic!("rules persist: {error}"));
        let permission_id = PermissionId::new("permission-1").expect("permission id");
        store
            .request_approval(&session_id, &permission_id, "tdw.udf.run", "tdw.udf.*")
            .await
            .unwrap_or_else(|error| panic!("approval persists: {error}"));
        let cost = CostLedgerEntry {
            session_id: session_id.as_str().to_string(),
            operation_id: "op-1".to_string(),
            tokens: 42,
            bytes_scanned: 2048,
            rows_read: 128,
            rows_written: 4,
            backend: "sqlite".to_string(),
        };
        store
            .append_cost(&cost)
            .await
            .unwrap_or_else(|error| panic!("cost entry persists: {error}"));

        assert_eq!(
            store
                .get_session(&session_id)
                .await
                .unwrap_or_else(|error| panic!("session loads: {error}"))
                .expect("session exists")
                .status,
            SessionStatus::Active
        );
        assert_eq!(
            store
                .load_permission_rules(&session_id)
                .await
                .unwrap_or_else(|error| panic!("rules load: {error}"))
                .expect("rules exist")
                .evaluate("tdw.query.run"),
            PermissionEffect::Allow
        );
        assert_eq!(
            store
                .resolve_approval(&permission_id, ApprovalDecision::AllowOnce)
                .await
                .unwrap_or_else(|error| panic!("approval resolves: {error}"))
                .expect("approval exists")
                .decision,
            Some(ApprovalDecision::AllowOnce)
        );
        assert_eq!(
            store
                .cost_entries(&session_id)
                .await
                .unwrap_or_else(|error| panic!("cost entries load: {error}")),
            vec![cost]
        );
    }

    #[tokio::test]
    async fn sqlite_store_rejects_invalid_persisted_enums() {
        let store = SqliteSessionStore::connect("sqlite::memory:")
            .await
            .unwrap_or_else(|error| panic!("store connects: {error}"));
        let session_id = SessionId::new("session-bad").expect("session id");
        sqlx::query(
            "insert into sessions (session_id, status, created_at, updated_at) values (?1, ?2, ?3, ?4)",
        )
        .bind(session_id.as_str())
        .bind("Paused")
        .bind("2026-05-22T00:00:00Z")
        .bind("2026-05-22T00:00:00Z")
        .execute(&store.pool)
        .await
        .unwrap_or_else(|error| panic!("invalid-status fixture inserts: {error}"));

        let status_error = store
            .get_session(&session_id)
            .await
            .expect_err("invalid session status should error");
        assert!(matches!(
            status_error,
            SessionError::InvalidSessionStatus(status) if status == "Paused"
        ));

        let permission_id = PermissionId::new("permission-bad").expect("permission id");
        sqlx::query(
            "insert into pending_approvals (permission_id, session_id, action, pattern, decision) values (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(permission_id.as_str())
        .bind(session_id.as_str())
        .bind("tdw.udf.run")
        .bind("tdw.udf.*")
        .bind("Maybe")
        .execute(&store.pool)
        .await
        .unwrap_or_else(|error| panic!("invalid-decision fixture inserts: {error}"));

        let decision_error = store
            .pending_approval(&permission_id)
            .await
            .expect_err("invalid approval decision should error");
        assert!(matches!(
            decision_error,
            SessionError::InvalidApprovalDecision(decision) if decision == "Maybe"
        ));
    }
}
