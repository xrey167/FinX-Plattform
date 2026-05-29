//! Postgres-backed outbox store for G013 durable persistence.
//!
//! Gated by the `postgres` feature. Built on top of
//! [`tdw_storage_postgres::PgEngine`] from G010 — no direct sqlx
//! usage in this crate. The schema is single-table:
//!
//! ```sql
//! CREATE TABLE tdw_outbox (
//!     sequence       BIGSERIAL PRIMARY KEY,
//!     envelope_json  JSONB NOT NULL,
//!     status         TEXT NOT NULL DEFAULT 'Pending',
//!     created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
//! );
//! ```
//!
//! The `Pending` / `Dispatched` literal strings match the existing
//! [`OutboxStatus`] enum's `Debug` output (Serialize emits the same
//! capitalised form). Custom tables can be configured via
//! [`PgOutboxStore::with_table`] for multi-tenant deployments.

use serde_json::{Value, json};
use tdw_core::{Error, RelationalEngine, Result};
use tdw_event::EventEnvelope;
use tdw_storage_postgres::PgEngine;

use crate::{OutboxRecord, OutboxStatus};

const DEFAULT_TABLE: &str = "tdw_outbox";

/// Production Postgres-backed outbox store. Cheap to clone (wraps a
/// `PgEngine`, itself a cheap clone of a `sqlx::PgPool`).
#[derive(Clone, Debug)]
pub struct PgOutboxStore {
    engine: PgEngine,
    table: String,
}

impl PgOutboxStore {
    /// Build a store against the supplied [`PgEngine`]. Uses
    /// `tdw_outbox` as the default table name.
    #[must_use]
    pub fn new(engine: PgEngine) -> Self {
        Self {
            engine,
            table: DEFAULT_TABLE.to_string(),
        }
    }

    /// Override the table name. Useful for multi-tenant deployments
    /// where each tenant has its own outbox table.
    #[must_use]
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = table.into();
        self
    }

    /// Idempotently create the outbox table. Callers should run this
    /// once at startup (or on first append).
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn ensure_schema(&self) -> Result<()> {
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                sequence BIGSERIAL PRIMARY KEY, \
                envelope_json JSONB NOT NULL, \
                status TEXT NOT NULL DEFAULT 'Pending', \
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()\
            )",
            self.table
        );
        self.engine.execute(&sql, Value::Null).await?;
        Ok(())
    }

    /// Append an event envelope. Returns the auto-assigned sequence
    /// number (matches the in-memory outbox contract).
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn append(&self, envelope: EventEnvelope<Value>) -> Result<u64> {
        let envelope_text = serde_json::to_string(&envelope)
            .map_err(|error| Error::Storage(format!("outbox encode envelope: {error}")))?;
        // CTE wrapper so PgEngine::fetch_json's row_to_json projection
        // can see RETURNING rows.
        let sql = format!(
            "WITH inserted AS (\
                INSERT INTO {} (envelope_json) VALUES ($1::jsonb) RETURNING sequence\
            ) SELECT sequence FROM inserted",
            self.table
        );
        let rows = self.engine.fetch_json(&sql, json!([envelope_text])).await?;
        let sequence = rows
            .first()
            .and_then(|row| row.get("sequence"))
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                Error::Storage("outbox append: missing sequence in INSERT RETURNING".to_string())
            })?;
        Ok(sequence as u64)
    }

    /// Mark the given sequence as dispatched. Returns `false` if no
    /// row matched (already dispatched or unknown sequence) — same
    /// semantics as `InMemoryOutbox::mark_dispatched`.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn mark_dispatched(&self, sequence: u64) -> Result<bool> {
        let sql = format!(
            "UPDATE {} SET status = 'Dispatched' WHERE sequence = $1 AND status = 'Pending'",
            self.table
        );
        let affected = self.engine.execute(&sql, json!([sequence as i64])).await?;
        Ok(affected > 0)
    }

    /// Fetch all pending records strictly after the supplied
    /// sequence, in ascending order. Empty `Vec` if nothing pending.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn pending_after(&self, sequence: u64) -> Result<Vec<OutboxRecord>> {
        let sql = format!(
            "SELECT sequence, envelope_json::text AS envelope_text \
             FROM {} \
             WHERE sequence > $1 AND status = 'Pending' \
             ORDER BY sequence ASC",
            self.table
        );
        let rows = self
            .engine
            .fetch_json(&sql, json!([sequence as i64]))
            .await?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let sequence = row
                .get("sequence")
                .and_then(Value::as_i64)
                .ok_or_else(|| Error::Storage("outbox pending: missing sequence".to_string()))?;
            let envelope_text = row
                .get("envelope_text")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    Error::Storage("outbox pending: missing envelope_text".to_string())
                })?;
            let envelope: EventEnvelope<Value> = serde_json::from_str(envelope_text)
                .map_err(|error| Error::Storage(format!("outbox decode envelope: {error}")))?;
            records.push(OutboxRecord {
                sequence: sequence as u64,
                envelope,
                status: OutboxStatus::Pending,
            });
        }
        Ok(records)
    }
}
