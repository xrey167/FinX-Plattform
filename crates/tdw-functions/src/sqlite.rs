//! SQLite-backed implementations of [`StepStore`] and [`RunStore`].
//!
//! Compiled only with the `sqlite` feature.
//!
//! Schema is created inline on construction via `CREATE TABLE IF NOT EXISTS`,
//! mirroring the pattern used by [`tdw_worker::SqliteWorkerQueue`].
//!
//! `put` / `insert_run` use `INSERT OR IGNORE` (SQLite's exactly-once
//! insert-or-ignore) to give the same first-write-wins semantics as the
//! [`super::step::InMemoryStepStore`] and the Postgres `ON CONFLICT DO NOTHING`
//! stores.

#![cfg(feature = "sqlite")]

use async_trait::async_trait;
use serde_json::Value;
use sqlx::Row;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};

use crate::error::FunctionError;
use crate::step::StepStore;
use crate::store::{RunRecord, RunStatus, RunStore};

// ---------------------------------------------------------------------------
// Migration SQL
// ---------------------------------------------------------------------------

const SQLITE_FUNCTIONS_MIGRATION: &str = r"
create table if not exists function_steps (
    run_id      text    not null,
    step_name   text    not null,
    result_json text    not null,
    created_at_ms integer not null,
    primary key (run_id, step_name)
);
create table if not exists function_runs (
    run_id        text    primary key,
    function_id   text    not null,
    payload       text    not null,
    status        text    not null,
    result        text,
    created_at_ms integer not null,
    updated_at_ms integer not null
)
";

// ---------------------------------------------------------------------------
// SqliteStepStore
// ---------------------------------------------------------------------------

/// SQLite-backed implementation of [`StepStore`].
///
/// Backed by an in-process SQLite file (or `:memory:` for tests).  Use
/// [`SqliteStepStore::connect`] to open via URL or
/// [`SqliteStepStore::from_pool`] to adopt an existing pool.
#[derive(Clone, Debug)]
pub struct SqliteStepStore {
    pool: SqlitePool,
}

impl SqliteStepStore {
    /// Open a connection pool against `database_url` and run the DDL migration.
    ///
    /// Passing `"sqlite::memory:"` creates an in-process ephemeral database,
    /// useful for tests.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the connection or migration fails.
    pub async fn connect(database_url: &str) -> Result<Self, FunctionError> {
        let options = database_url
            .parse::<SqliteConnectOptions>()
            .map_err(|e| FunctionError::Store(format!("sqlite options: {e}")))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| FunctionError::Store(format!("sqlite connect: {e}")))?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Adopt a caller-built pool (schema migration is still applied).
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the migration fails.
    pub async fn from_pool(pool: SqlitePool) -> Result<Self, FunctionError> {
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), FunctionError> {
        for statement in SQLITE_FUNCTIONS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(|e| FunctionError::Store(format!("migrate: {e}")))?;
        }
        Ok(())
    }
}

#[async_trait]
impl StepStore for SqliteStepStore {
    async fn get(&self, run_id: &str, step: &str) -> Result<Option<Value>, FunctionError> {
        let row: Option<SqliteRow> = sqlx::query(
            "select result_json from function_steps where run_id = ?1 and step_name = ?2 limit 1",
        )
        .bind(run_id)
        .bind(step)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| FunctionError::Store(format!("step get: {e}")))?;

        match row {
            None => Ok(None),
            Some(row) => {
                let text: &str = row.get("result_json");
                serde_json::from_str(text)
                    .map(Some)
                    .map_err(|e| FunctionError::Serialization(e.to_string()))
            }
        }
    }

    async fn put(&self, run_id: &str, step: &str, value: &Value) -> Result<(), FunctionError> {
        let json_text = serde_json::to_string(value)
            .map_err(|e| FunctionError::Serialization(e.to_string()))?;
        let now_ms = now_ms_i64();
        sqlx::query(
            r"
            insert or ignore into function_steps (run_id, step_name, result_json, created_at_ms)
            values (?1, ?2, ?3, ?4)
            ",
        )
        .bind(run_id)
        .bind(step)
        .bind(json_text)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(|e| FunctionError::Store(format!("step put: {e}")))?;
        Ok(())
    }

    async fn list(&self, run_id: &str) -> Result<Vec<(String, Value)>, FunctionError> {
        let rows: Vec<SqliteRow> = sqlx::query(
            "select step_name, result_json from function_steps where run_id = ?1 order by step_name asc",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| FunctionError::Store(format!("step list: {e}")))?;

        rows.iter()
            .map(|row| {
                let step_name: String = row.get("step_name");
                let text: &str = row.get("result_json");
                let value = serde_json::from_str(text)
                    .map_err(|e| FunctionError::Serialization(e.to_string()))?;
                Ok((step_name, value))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// SqliteRunStore
// ---------------------------------------------------------------------------

/// SQLite-backed implementation of [`RunStore`].
///
/// Shares the same database as [`SqliteStepStore`] when both are constructed
/// from the same pool or URL; the two tables are independent.
#[derive(Clone, Debug)]
pub struct SqliteRunStore {
    pool: SqlitePool,
}

impl SqliteRunStore {
    /// Open a connection pool against `database_url` and run the DDL migration.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the connection or migration fails.
    pub async fn connect(database_url: &str) -> Result<Self, FunctionError> {
        let options = database_url
            .parse::<SqliteConnectOptions>()
            .map_err(|e| FunctionError::Store(format!("sqlite options: {e}")))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|e| FunctionError::Store(format!("sqlite connect: {e}")))?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Adopt a caller-built pool (schema migration is still applied).
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the migration fails.
    pub async fn from_pool(pool: SqlitePool) -> Result<Self, FunctionError> {
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), FunctionError> {
        for statement in SQLITE_FUNCTIONS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(|e| FunctionError::Store(format!("migrate: {e}")))?;
        }
        Ok(())
    }
}

#[async_trait]
impl RunStore for SqliteRunStore {
    async fn insert_run(&self, record: &RunRecord) -> Result<(), FunctionError> {
        let payload_json = serde_json::to_string(&record.payload)
            .map_err(|e| FunctionError::Serialization(e.to_string()))?;
        sqlx::query(
            r"
            insert or ignore into function_runs
                (run_id, function_id, payload, status, result, created_at_ms, updated_at_ms)
            values (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
        )
        .bind(&record.run_id)
        .bind(&record.function_id)
        .bind(payload_json)
        .bind(record.status.as_str())
        .bind(Option::<String>::None)
        .bind(record.created_at_ms)
        .bind(record.updated_at_ms)
        .execute(&self.pool)
        .await
        .map_err(|e| FunctionError::Store(format!("insert_run: {e}")))?;
        Ok(())
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecord>, FunctionError> {
        let row: Option<SqliteRow> = sqlx::query(
            r"
            select run_id, function_id, payload, status, result, created_at_ms, updated_at_ms
            from function_runs
            where run_id = ?1
            limit 1
            ",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| FunctionError::Store(format!("get_run: {e}")))?;

        match row {
            None => Ok(None),
            Some(row) => {
                let payload_text: &str = row.get("payload");
                let payload: Value = serde_json::from_str(payload_text)
                    .map_err(|e| FunctionError::Serialization(e.to_string()))?;
                let status_str: &str = row.get("status");
                let status = RunStatus::parse(status_str)?;
                let result_text: Option<String> = row.get("result");
                let result = result_text
                    .map(|text| {
                        serde_json::from_str::<Value>(&text)
                            .map_err(|e| FunctionError::Serialization(e.to_string()))
                    })
                    .transpose()?;
                Ok(Some(RunRecord {
                    run_id: row.get("run_id"),
                    function_id: row.get("function_id"),
                    payload,
                    status,
                    result,
                    created_at_ms: row.get("created_at_ms"),
                    updated_at_ms: row.get("updated_at_ms"),
                }))
            }
        }
    }

    async fn set_status(
        &self,
        run_id: &str,
        status: RunStatus,
        result: Option<Value>,
        updated_at_ms: i64,
    ) -> Result<(), FunctionError> {
        let result_json = result
            .map(|v| {
                serde_json::to_string(&v).map_err(|e| FunctionError::Serialization(e.to_string()))
            })
            .transpose()?;
        sqlx::query(
            r"
            update function_runs
            set status = ?1, result = ?2, updated_at_ms = ?3
            where run_id = ?4
            ",
        )
        .bind(status.as_str())
        .bind(result_json)
        .bind(updated_at_ms)
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(|e| FunctionError::Store(format!("set_status: {e}")))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn now_ms_i64() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
