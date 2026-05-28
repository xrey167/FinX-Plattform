//! Postgres-backed snapshot store for G013 durable persistence.
//!
//! Gated by the `postgres` feature. Built on top of
//! [`tdw_storage_postgres::PgEngine`] from G010.
//!
//! Schema:
//!
//! ```sql
//! CREATE TABLE tdw_snapshot (
//!     id          BIGSERIAL PRIMARY KEY,
//!     table_name  TEXT NOT NULL,
//!     version     BIGINT NOT NULL,
//!     created_at  TEXT NOT NULL,
//!     row_ids     JSONB NOT NULL,
//!     UNIQUE (table_name, version)
//! );
//! ```
//!
//! The `(table_name, version)` UNIQUE constraint serialises concurrent
//! `commit` callers under contention; the caller-visible semantics
//! match the in-memory [`crate::SnapshotStore`] (monotonic per-table
//! version starting at 1).

use serde_json::{Value, json};
use tdw_core::{Error, RelationalEngine, Result};
use tdw_storage_postgres::PgEngine;

use crate::Snapshot;

const DEFAULT_TABLE: &str = "tdw_snapshot";

/// Production Postgres-backed snapshot store. Cheap to clone (wraps
/// a `PgEngine`, itself a cheap clone of a `sqlx::PgPool`).
#[derive(Clone, Debug)]
pub struct PgSnapshotStore {
    engine: PgEngine,
    table: String,
}

impl PgSnapshotStore {
    /// Build a store against the supplied [`PgEngine`]. Uses
    /// `tdw_snapshot` as the default table name.
    #[must_use]
    pub fn new(engine: PgEngine) -> Self {
        Self {
            engine,
            table: DEFAULT_TABLE.to_string(),
        }
    }

    /// Override the table name. Useful for multi-tenant deployments.
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = table.into();
        self
    }

    /// Idempotently create the snapshot table + uniqueness index.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn ensure_schema(&self) -> Result<()> {
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                id BIGSERIAL PRIMARY KEY, \
                table_name TEXT NOT NULL, \
                version BIGINT NOT NULL, \
                created_at TEXT NOT NULL, \
                row_ids JSONB NOT NULL, \
                UNIQUE (table_name, version)\
            )",
            self.table
        );
        self.engine.execute(&sql, Value::Null).await?;
        Ok(())
    }

    /// Commit a new snapshot for `table`. The version is computed
    /// server-side as `MAX(version) + 1` against existing rows for
    /// the same table; under contention the UNIQUE constraint will
    /// reject the loser of a race so callers may need to retry on
    /// `Error::Storage`. The returned [`Snapshot`] reflects the
    /// row that actually persisted.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn commit(
        &self,
        table: impl Into<String>,
        created_at: impl Into<String>,
        row_ids: Vec<String>,
    ) -> Result<Snapshot> {
        let table_name = table.into();
        let created_at = created_at.into();
        let row_ids_json = serde_json::to_string(&row_ids)
            .map_err(|error| Error::Storage(format!("snapshot encode row_ids: {error}")))?;
        let sql = format!(
            "WITH inserted AS (\
                INSERT INTO {table} (table_name, version, created_at, row_ids) \
                SELECT $1, COALESCE(MAX(version), 0) + 1, $2, $3::jsonb \
                FROM {table} WHERE table_name = $1 \
                RETURNING table_name, version, created_at, row_ids::text AS row_ids_text\
            ) SELECT table_name, version, created_at, row_ids_text FROM inserted",
            table = self.table
        );
        let rows = self
            .engine
            .fetch_json(&sql, json!([table_name, created_at, row_ids_json]))
            .await?;
        let row = rows.first().ok_or_else(|| {
            Error::Storage("snapshot commit: INSERT RETURNING produced no rows".to_string())
        })?;
        Snapshot::try_from_row(row)
    }

    /// Look up a specific version of `table`. Returns `None` if the
    /// version has not been committed.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn as_of_version(&self, table: &str, version: u64) -> Result<Option<Snapshot>> {
        let sql = format!(
            "SELECT table_name, version, created_at, row_ids::text AS row_ids_text \
             FROM {} WHERE table_name = $1 AND version = $2 LIMIT 1",
            self.table
        );
        let rows = self
            .engine
            .fetch_json(&sql, json!([table, version as i64]))
            .await?;
        match rows.first() {
            Some(row) => Snapshot::try_from_row(row).map(Some),
            None => Ok(None),
        }
    }

    /// Look up the latest version of `table`. Returns `None` if no
    /// snapshot has been committed for the table.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn latest(&self, table: &str) -> Result<Option<Snapshot>> {
        let sql = format!(
            "SELECT table_name, version, created_at, row_ids::text AS row_ids_text \
             FROM {} WHERE table_name = $1 ORDER BY version DESC LIMIT 1",
            self.table
        );
        let rows = self.engine.fetch_json(&sql, json!([table])).await?;
        match rows.first() {
            Some(row) => Snapshot::try_from_row(row).map(Some),
            None => Ok(None),
        }
    }
}

impl Snapshot {
    fn try_from_row(row: &Value) -> Result<Self> {
        let table = row
            .get("table_name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Storage("snapshot row: missing table_name".to_string()))?
            .to_string();
        let version = row
            .get("version")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Storage("snapshot row: missing version".to_string()))?
            as u64;
        let created_at = row
            .get("created_at")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Storage("snapshot row: missing created_at".to_string()))?
            .to_string();
        let row_ids_text = row
            .get("row_ids_text")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Storage("snapshot row: missing row_ids_text".to_string()))?;
        let row_ids: Vec<String> = serde_json::from_str(row_ids_text)
            .map_err(|error| Error::Storage(format!("snapshot decode row_ids: {error}")))?;
        Ok(Self {
            table,
            version,
            created_at,
            row_ids,
        })
    }
}
