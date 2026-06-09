//! Postgres-backed implementations of [`StepStore`] and [`RunStore`].
//!
//! Compiled only with the `postgres` feature.

#![cfg(feature = "postgres")]

use async_trait::async_trait;
use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::error::FunctionError;
use crate::step::StepStore;
use crate::store::{RunRecord, RunStatus, RunStore};

// ---------------------------------------------------------------------------
// Migration SQL (inlined)
// ---------------------------------------------------------------------------

/// SQL included from the migration file — used by the integration test setup.
pub const PG_FUNCTIONS_MIGRATION: &str =
    include_str!("../../../migrations/postgres/20260607_0002_function_steps.sql");

// ---------------------------------------------------------------------------
// PgStepStore
// ---------------------------------------------------------------------------

/// Postgres-backed implementation of [`StepStore`].
///
/// Table: `system.function_steps` (created by migration `20260607_0002`).
///
/// `put` uses `ON CONFLICT DO NOTHING` for exactly-once first-write-wins
/// semantics.
#[derive(Clone, Debug)]
pub struct PgStepStore {
    pool: PgPool,
}

impl PgStepStore {
    /// Open a connection pool against `database_url`.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the connection fails.
    pub async fn connect(database_url: &str) -> Result<Self, FunctionError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| FunctionError::Store(format!("postgres connect: {error}")))?;
        Ok(Self { pool })
    }

    /// Adopt a caller-built pool.
    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Expose the underlying pool for callers that need raw sqlx access.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Apply the migration DDL.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if a DDL statement fails.
    pub async fn migrate(&self) -> Result<(), FunctionError> {
        for statement in PG_FUNCTIONS_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            sqlx::query(statement)
                .execute(&self.pool)
                .await
                .map_err(|error| FunctionError::Store(format!("migrate: {error}")))?;
        }
        Ok(())
    }
}

#[async_trait]
impl StepStore for PgStepStore {
    async fn get(
        &self,
        run_id: &str,
        step: &str,
    ) -> Result<Option<serde_json::Value>, FunctionError> {
        let row = sqlx::query(
            r#"
            select result_json::text as result_json
            from system.function_steps
            where run_id = $1 and step_name = $2
            limit 1
            "#,
        )
        .bind(run_id)
        .bind(step)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| FunctionError::Store(format!("step get: {error}")))?;

        match row {
            None => Ok(None),
            Some(row) => {
                let json_text: &str = row.get("result_json");
                let value = serde_json::from_str(json_text)
                    .map_err(|error| FunctionError::Serialization(error.to_string()))?;
                Ok(Some(value))
            }
        }
    }

    async fn put(
        &self,
        run_id: &str,
        step: &str,
        value: &serde_json::Value,
    ) -> Result<(), FunctionError> {
        let json_text = serde_json::to_string(value)
            .map_err(|error| FunctionError::Serialization(error.to_string()))?;
        sqlx::query(
            r#"
            insert into system.function_steps (run_id, step_name, result_json, created_at_ms)
            values ($1, $2, $3::jsonb, $4)
            on conflict (run_id, step_name) do nothing
            "#,
        )
        .bind(run_id)
        .bind(step)
        .bind(json_text)
        .bind(now_ms())
        .execute(&self.pool)
        .await
        .map_err(|error| FunctionError::Store(format!("step put: {error}")))?;
        Ok(())
    }

    async fn list(&self, run_id: &str) -> Result<Vec<(String, serde_json::Value)>, FunctionError> {
        let rows = sqlx::query(
            r#"
            select step_name, result_json::text as result_json
            from system.function_steps
            where run_id = $1
            order by step_name asc
            "#,
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| FunctionError::Store(format!("step list: {error}")))?;

        rows.iter()
            .map(|row| {
                let step_name: String = row.get("step_name");
                let json_text: &str = row.get("result_json");
                let value = serde_json::from_str(json_text)
                    .map_err(|error| FunctionError::Serialization(error.to_string()))?;
                Ok((step_name, value))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// PgRunStore
// ---------------------------------------------------------------------------

/// Postgres-backed implementation of [`RunStore`].
///
/// Table: `system.function_runs` (created by migration `20260607_0002`).
#[derive(Clone, Debug)]
pub struct PgRunStore {
    pool: PgPool,
}

impl PgRunStore {
    /// Open a connection pool against `database_url`.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the connection fails.
    pub async fn connect(database_url: &str) -> Result<Self, FunctionError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|error| FunctionError::Store(format!("postgres connect: {error}")))?;
        Ok(Self { pool })
    }

    /// Adopt a caller-built pool.
    #[must_use]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Expose the underlying pool for callers that need raw sqlx access.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl RunStore for PgRunStore {
    async fn insert_run(&self, record: &RunRecord) -> Result<(), FunctionError> {
        let payload_json = serde_json::to_string(&record.payload)
            .map_err(|error| FunctionError::Serialization(error.to_string()))?;
        sqlx::query(
            r#"
            insert into system.function_runs (
                run_id, function_id, payload, status,
                result, created_at_ms, updated_at_ms
            )
            values ($1, $2, $3::jsonb, $4, $5::jsonb, $6, $7)
            "#,
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
        .map_err(|error| FunctionError::Store(format!("insert_run: {error}")))?;
        Ok(())
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecord>, FunctionError> {
        let row = sqlx::query(
            r#"
            select run_id, function_id,
                   payload::text as payload,
                   status,
                   result::text as result,
                   created_at_ms, updated_at_ms
            from system.function_runs
            where run_id = $1
            limit 1
            "#,
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| FunctionError::Store(format!("get_run: {error}")))?;

        match row {
            None => Ok(None),
            Some(row) => {
                let payload_text: &str = row.get("payload");
                let payload: serde_json::Value = serde_json::from_str(payload_text)
                    .map_err(|error| FunctionError::Serialization(error.to_string()))?;
                let status_str: &str = row.get("status");
                let status = RunStatus::parse(status_str)?;
                let result_text: Option<&str> = row.get("result");
                let result = result_text
                    .map(|text| {
                        serde_json::from_str(text)
                            .map_err(|error| FunctionError::Serialization(error.to_string()))
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
        result: Option<serde_json::Value>,
        updated_at_ms: i64,
    ) -> Result<(), FunctionError> {
        let result_json = result
            .map(|value| {
                serde_json::to_string(&value)
                    .map_err(|error| FunctionError::Serialization(error.to_string()))
            })
            .transpose()?;
        sqlx::query(
            r#"
            update system.function_runs
            set status = $1,
                result = $2::jsonb,
                updated_at_ms = $3
            where run_id = $4
            "#,
        )
        .bind(status.as_str())
        .bind(result_json)
        .bind(updated_at_ms)
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(|error| FunctionError::Store(format!("set_status: {error}")))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0) as i64
}
