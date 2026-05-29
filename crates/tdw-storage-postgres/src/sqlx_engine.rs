//! Real Postgres backend for `tdw_core::RelationalEngine`.
//!
//! Gated by the `postgres` feature. Wraps [`sqlx::PgPool`] and dispatches
//! `execute` / `fetch_json` directly to Postgres. Result rows are converted
//! to `serde_json::Value` server-side via `row_to_json`, so any SELECT
//! shape works without per-column decode plumbing in this crate.
//!
//! Parameter binding is intentionally narrow in this slice: callers can
//! pass a JSON `null` (no params) or a JSON array of primitives (null,
//! bool, integer, float, string). Object-shaped params or nested
//! structures are rejected with a clear error so production callers know
//! to extend the binding surface deliberately.

use async_trait::async_trait;
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions};
use tdw_core::{Error, RelationalEngine, Result};

/// Production Postgres backend. Construct via [`PgEngine::connect`] (URL)
/// or [`PgEngine::with_pool`] (pre-built pool).
#[derive(Clone, Debug)]
pub struct PgEngine {
    pool: PgPool,
}

impl PgEngine {
    /// Open a connection pool against `database_url` (e.g.
    /// `postgres://user:pass@localhost:5432/db`).
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(database_url)
            .await
            .map_err(|error| Error::Storage(format!("postgres connect: {error}")))?;
        Ok(Self { pool })
    }

    /// Adopt a caller-built pool (useful for testcontainers or shared
    /// pools).
    #[must_use]
    pub const fn with_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Expose the underlying pool for code that needs sqlx APIs directly.
    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl RelationalEngine for PgEngine {
    async fn execute(&self, sql: &str, params: Value) -> Result<u64> {
        validate_sql(sql)?;
        let bindings = decode_bindings(params)?;
        // sqlx 0.9 requires &'static str for `sqlx::query`; QueryBuilder
        // accepts dynamic strings and produces an equivalent Query.
        let mut builder = sqlx::QueryBuilder::<sqlx::Postgres>::new(sql);
        let mut query = builder.build();
        for binding in bindings {
            query = bind_one(query, binding);
        }
        let result = query
            .execute(&self.pool)
            .await
            .map_err(|error| Error::Storage(format!("postgres execute: {error}")))?;
        Ok(result.rows_affected())
    }

    async fn fetch_json(&self, sql: &str, params: Value) -> Result<Vec<Value>> {
        validate_sql(sql)?;
        let bindings = decode_bindings(params)?;
        // Wrap the caller's SELECT so Postgres does the per-row JSON
        // conversion. This avoids per-column decode plumbing for the MVP
        // and supports any output column shape Postgres can serialise.
        let wrapped =
            format!("SELECT row_to_json(__tdw_t)::text AS __tdw_json FROM ({sql}) __tdw_t");
        let mut builder = sqlx::QueryBuilder::<sqlx::Postgres>::new(wrapped);
        let mut query = builder.build();
        for binding in bindings {
            query = bind_one(query, binding);
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|error| Error::Storage(format!("postgres fetch_json: {error}")))?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let text: String = row
                .try_get::<String, _>("__tdw_json")
                .map_err(|error| Error::Storage(format!("postgres row_to_json: {error}")))?;
            let value = serde_json::from_str(&text)
                .map_err(|error| Error::Storage(format!("postgres parse_json: {error}")))?;
            out.push(value);
        }
        Ok(out)
    }
}

fn validate_sql(sql: &str) -> Result<()> {
    if sql.trim().is_empty() {
        return Err(Error::Storage("postgres sql must not be empty".to_string()));
    }
    Ok(())
}

enum Binding {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

fn decode_bindings(params: Value) -> Result<Vec<Binding>> {
    match params {
        Value::Null => Ok(Vec::new()),
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(decode_one(item)?);
            }
            Ok(out)
        }
        Value::Object(_) | Value::String(_) | Value::Number(_) | Value::Bool(_) => {
            Err(Error::Storage(
                "postgres params must be JSON null or an array of primitives".to_string(),
            ))
        }
    }
}

fn decode_one(value: Value) -> Result<Binding> {
    match value {
        Value::Null => Ok(Binding::Null),
        Value::Bool(boolean) => Ok(Binding::Bool(boolean)),
        Value::Number(number) => number.as_i64().map_or_else(
            || {
                number.as_f64().map_or_else(
                    || {
                        Err(Error::Storage(format!(
                            "postgres param: unrepresentable number {number}"
                        )))
                    },
                    |float| Ok(Binding::Float(float)),
                )
            },
            |int| Ok(Binding::Int(int)),
        ),
        Value::String(text) => Ok(Binding::Text(text)),
        Value::Array(_) | Value::Object(_) => Err(Error::Storage(
            "postgres param: nested arrays and objects are not supported in this slice".to_string(),
        )),
    }
}

fn bind_one<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    binding: Binding,
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    match binding {
        Binding::Null => query.bind(Option::<String>::None),
        Binding::Bool(boolean) => query.bind(boolean),
        Binding::Int(int) => query.bind(int),
        Binding::Float(float) => query.bind(float),
        Binding::Text(text) => query.bind(text),
    }
}
