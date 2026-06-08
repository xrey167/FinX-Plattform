//! Postgres-backed session store for G013 durable persistence.
//!
//! Gated by the `postgres` feature. Built on top of
//! [`tdw_storage_postgres::PgEngine`] (G010). Provides a full
//! Postgres mirror of [`crate::SqliteSessionStore`]:
//!
//!   - core session CRUD (`upsert_session`, `get_session`)
//!   - permission rule upsert / load (`save_permission_rules`,
//!     `load_permission_rules`)
//!   - approval request / resolve / lookup (`request_approval`,
//!     `resolve_approval`, `pending_approval`)
//!   - cost ledger append + query (`append_cost`, `cost_entries`)
//!
//! Coexists with the existing sqlite store; deployments choose at
//! runtime via the constructor.
//!
//! Schema (four tables, all prefixed by the base name supplied to
//! [`PgSessionStore::with_table`], default `tdw_sessions`):
//!
//! ```sql
//! CREATE TABLE tdw_sessions                  (session_id TEXT PK, status TEXT, created_at TEXT, updated_at TEXT);
//! CREATE TABLE tdw_sessions_permission_state (session_id TEXT PK, rules_json TEXT);
//! CREATE TABLE tdw_sessions_pending_approvals(permission_id TEXT PK, session_id TEXT, action TEXT, pattern TEXT, decision TEXT NULL);
//! CREATE TABLE tdw_sessions_cost_ledger      (id BIGSERIAL PK, session_id TEXT, operation_id TEXT, tokens BIGINT, bytes_scanned BIGINT, rows_read BIGINT, rows_written BIGINT, backend TEXT, UNIQUE(session_id, operation_id));
//! ```

use serde_json::{Value, json};
use tdw_core::{Error, RelationalEngine};
use tdw_hooks::PermissionRules;
use tdw_protocol::{ApprovalDecision, PermissionId, SessionId};
use tdw_storage_postgres::PgEngine;

use crate::{CostLedgerEntry, PendingApprovalRecord, SessionRecord, SessionStatus};

const DEFAULT_TABLE: &str = "tdw_sessions";

/// Production Postgres-backed session store. Wraps a `PgEngine`.
#[derive(Clone, Debug)]
pub struct PgSessionStore {
    engine: PgEngine,
    table: String,
}

impl PgSessionStore {
    /// Build a store against the supplied [`PgEngine`]. Defaults to
    /// `tdw_sessions` as the base table; companion tables
    /// (`<base>_permission_state`, `<base>_pending_approvals`,
    /// `<base>_cost_ledger`) are derived by suffix.
    #[must_use]
    pub fn new(engine: PgEngine) -> Self {
        Self {
            engine,
            table: DEFAULT_TABLE.to_string(),
        }
    }

    /// Override the base table name. Companion tables follow the
    /// `<base>_<suffix>` convention.
    #[must_use]
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = table.into();
        self
    }

    fn permission_table(&self) -> String {
        format!("{}_permission_state", self.table)
    }

    fn approval_table(&self) -> String {
        format!("{}_pending_approvals", self.table)
    }

    fn cost_table(&self) -> String {
        format!("{}_cost_ledger", self.table)
    }

    /// Idempotently create all four backing tables.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn ensure_schema(&self) -> Result<(), Error> {
        let sessions_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                session_id TEXT PRIMARY KEY, \
                status TEXT NOT NULL, \
                created_at TEXT NOT NULL, \
                updated_at TEXT NOT NULL\
            )",
            self.table
        );
        self.engine.execute(&sessions_sql, Value::Null).await?;

        let perm_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                session_id TEXT PRIMARY KEY, \
                rules_json TEXT NOT NULL\
            )",
            self.permission_table()
        );
        self.engine.execute(&perm_sql, Value::Null).await?;

        let approval_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                permission_id TEXT PRIMARY KEY, \
                session_id TEXT NOT NULL, \
                action TEXT NOT NULL, \
                pattern TEXT NOT NULL, \
                decision TEXT\
            )",
            self.approval_table()
        );
        self.engine.execute(&approval_sql, Value::Null).await?;

        let cost_sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                id BIGSERIAL PRIMARY KEY, \
                session_id TEXT NOT NULL, \
                operation_id TEXT NOT NULL, \
                tokens BIGINT NOT NULL, \
                bytes_scanned BIGINT NOT NULL, \
                rows_read BIGINT NOT NULL, \
                rows_written BIGINT NOT NULL, \
                backend TEXT NOT NULL, \
                UNIQUE (session_id, operation_id)\
            )",
            self.cost_table()
        );
        self.engine.execute(&cost_sql, Value::Null).await?;

        Ok(())
    }

    // ─── session CRUD ────────────────────────────────────────────

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn upsert_session(&self, record: &SessionRecord) -> Result<(), Error> {
        let sql = format!(
            "INSERT INTO {table} (session_id, status, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (session_id) DO UPDATE SET \
                status = EXCLUDED.status, \
                updated_at = EXCLUDED.updated_at",
            table = self.table
        );
        self.engine
            .execute(
                &sql,
                json!([
                    record.session_id,
                    status_to_text(record.status),
                    record.created_at,
                    record.updated_at
                ]),
            )
            .await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<SessionRecord>, Error> {
        let sql = format!(
            "SELECT session_id, status, created_at, updated_at FROM {} \
             WHERE session_id = $1 LIMIT 1",
            self.table
        );
        let rows = self
            .engine
            .fetch_json(&sql, json!([session_id.as_str()]))
            .await?;
        match rows.first() {
            Some(row) => Ok(Some(row_to_session(row)?)),
            None => Ok(None),
        }
    }

    // ─── permission rules ────────────────────────────────────────

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn save_permission_rules(
        &self,
        session_id: &SessionId,
        rules: &PermissionRules,
    ) -> Result<(), Error> {
        let rules_json = serde_json::to_string(rules)
            .map_err(|error| Error::Storage(format!("encode permission rules: {error}")))?;
        let sql = format!(
            "INSERT INTO {table} (session_id, rules_json) \
             VALUES ($1, $2) \
             ON CONFLICT (session_id) DO UPDATE SET rules_json = EXCLUDED.rules_json",
            table = self.permission_table()
        );
        self.engine
            .execute(&sql, json!([session_id.as_str(), rules_json]))
            .await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn load_permission_rules(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<PermissionRules>, Error> {
        let sql = format!(
            "SELECT rules_json FROM {} WHERE session_id = $1 LIMIT 1",
            self.permission_table()
        );
        let rows = self
            .engine
            .fetch_json(&sql, json!([session_id.as_str()]))
            .await?;
        match rows.first() {
            Some(row) => {
                let text = row
                    .get("rules_json")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        Error::Storage("permission row: missing rules_json".to_string())
                    })?;
                let rules: PermissionRules = serde_json::from_str(text)
                    .map_err(|error| Error::Storage(format!("decode permission rules: {error}")))?;
                Ok(Some(rules))
            }
            None => Ok(None),
        }
    }

    // ─── approvals ───────────────────────────────────────────────

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn request_approval(
        &self,
        session_id: &SessionId,
        permission_id: &PermissionId,
        action: &str,
        pattern: &str,
    ) -> Result<(), Error> {
        let sql = format!(
            "INSERT INTO {} (permission_id, session_id, action, pattern, decision) \
             VALUES ($1, $2, $3, $4, NULL)",
            self.approval_table()
        );
        self.engine
            .execute(
                &sql,
                json!([permission_id.as_str(), session_id.as_str(), action, pattern]),
            )
            .await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn resolve_approval(
        &self,
        permission_id: &PermissionId,
        decision: ApprovalDecision,
    ) -> Result<Option<PendingApprovalRecord>, Error> {
        // Mirror the sqlite store's first-decision-wins guard: `decision IS NULL`
        // prevents a late caller from silently overwriting a resolved approval.
        let sql = format!(
            "UPDATE {} SET decision = $1 WHERE permission_id = $2 AND decision IS NULL",
            self.approval_table()
        );
        self.engine
            .execute(
                &sql,
                json!([decision_to_text(decision), permission_id.as_str()]),
            )
            .await?;
        self.pending_approval(permission_id).await
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn pending_approval(
        &self,
        permission_id: &PermissionId,
    ) -> Result<Option<PendingApprovalRecord>, Error> {
        let sql = format!(
            "SELECT permission_id, session_id, action, pattern, decision \
             FROM {} WHERE permission_id = $1 LIMIT 1",
            self.approval_table()
        );
        let rows = self
            .engine
            .fetch_json(&sql, json!([permission_id.as_str()]))
            .await?;
        match rows.first() {
            Some(row) => Ok(Some(row_to_pending_approval(row)?)),
            None => Ok(None),
        }
    }

    // ─── cost ledger ─────────────────────────────────────────────

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn append_cost(&self, entry: &CostLedgerEntry) -> Result<(), Error> {
        let sql = format!(
            "INSERT INTO {} (session_id, operation_id, tokens, bytes_scanned, rows_read, rows_written, backend) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (session_id, operation_id) DO NOTHING",
            self.cost_table()
        );
        self.engine
            .execute(
                &sql,
                json!([
                    entry.session_id,
                    entry.operation_id,
                    entry.tokens,
                    entry.bytes_scanned,
                    entry.rows_read,
                    entry.rows_written,
                    entry.backend
                ]),
            )
            .await?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn cost_entries(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<CostLedgerEntry>, Error> {
        let sql = format!(
            "SELECT session_id, operation_id, tokens, bytes_scanned, rows_read, rows_written, backend \
             FROM {} WHERE session_id = $1 ORDER BY id ASC",
            self.cost_table()
        );
        let rows = self
            .engine
            .fetch_json(&sql, json!([session_id.as_str()]))
            .await?;
        rows.iter().map(row_to_cost_entry).collect()
    }
}

// ─── row decoders ────────────────────────────────────────────────

fn row_to_session(row: &Value) -> Result<SessionRecord, Error> {
    Ok(SessionRecord {
        session_id: pull_str(row, "session_id")?.to_string(),
        status: parse_status(pull_str(row, "status")?)?,
        created_at: pull_str(row, "created_at")?.to_string(),
        updated_at: pull_str(row, "updated_at")?.to_string(),
    })
}

fn row_to_pending_approval(row: &Value) -> Result<PendingApprovalRecord, Error> {
    let decision = match row.get("decision") {
        Some(Value::Null) | None => None,
        Some(Value::String(text)) => Some(parse_decision(text)?),
        Some(other) => {
            return Err(Error::Storage(format!(
                "approval row: decision must be string or null, got {other}"
            )));
        }
    };
    Ok(PendingApprovalRecord {
        permission_id: pull_str(row, "permission_id")?.to_string(),
        session_id: pull_str(row, "session_id")?.to_string(),
        action: pull_str(row, "action")?.to_string(),
        pattern: pull_str(row, "pattern")?.to_string(),
        decision,
    })
}

fn row_to_cost_entry(row: &Value) -> Result<CostLedgerEntry, Error> {
    Ok(CostLedgerEntry {
        session_id: pull_str(row, "session_id")?.to_string(),
        operation_id: pull_str(row, "operation_id")?.to_string(),
        tokens: pull_i64(row, "tokens")?,
        bytes_scanned: pull_i64(row, "bytes_scanned")?,
        rows_read: pull_i64(row, "rows_read")?,
        rows_written: pull_i64(row, "rows_written")?,
        backend: pull_str(row, "backend")?.to_string(),
    })
}

fn pull_str<'a>(row: &'a Value, field: &str) -> Result<&'a str, Error> {
    row.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Storage(format!("session row: missing or non-string {field}")))
}

fn pull_i64(row: &Value, field: &str) -> Result<i64, Error> {
    row.get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::Storage(format!("session row: missing or non-int {field}")))
}

const fn status_to_text(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "Active",
        SessionStatus::Completed => "Completed",
        SessionStatus::Failed => "Failed",
    }
}

fn parse_status(text: &str) -> Result<SessionStatus, Error> {
    match text {
        "Active" => Ok(SessionStatus::Active),
        "Completed" => Ok(SessionStatus::Completed),
        "Failed" => Ok(SessionStatus::Failed),
        other => Err(Error::Storage(format!(
            "session row: unknown status {other:?}"
        ))),
    }
}

const fn decision_to_text(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::AllowOnce => "AllowOnce",
        ApprovalDecision::AlwaysAllow => "AlwaysAllow",
        ApprovalDecision::Deny => "Deny",
    }
}

fn parse_decision(text: &str) -> Result<ApprovalDecision, Error> {
    match text {
        "AllowOnce" => Ok(ApprovalDecision::AllowOnce),
        "AlwaysAllow" => Ok(ApprovalDecision::AlwaysAllow),
        "Deny" => Ok(ApprovalDecision::Deny),
        other => Err(Error::Storage(format!(
            "approval row: unknown decision {other:?}"
        ))),
    }
}
