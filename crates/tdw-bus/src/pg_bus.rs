//! Postgres-backed event bus for G013 durable persistence.
//!
//! Gated by the `postgres` feature. Built on top of
//! [`tdw_storage_postgres::PgEngine`] from G010.
//!
//! Schema:
//!
//! ```sql
//! CREATE TABLE tdw_bus (
//!     sequence       BIGSERIAL PRIMARY KEY,
//!     envelope_json  JSONB NOT NULL,
//!     created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
//! );
//! ```
//!
//! The Postgres backend does not implement the in-memory bus'
//! capacity-based eviction (the equivalent is a separate
//! housekeeping task that prunes rows older than a retention
//! window). Callers that need bounded retention should run a
//! periodic `DELETE FROM tdw_bus WHERE sequence < threshold`
//! out-of-band.

use serde_json::{Value, json};
use tdw_core::{Error, RelationalEngine, Result};
use tdw_event::EventEnvelope;

use crate::{BusEntry, RetentionWindow};
use tdw_storage_postgres::PgEngine;

const DEFAULT_TABLE: &str = "tdw_bus";

/// Production Postgres-backed event bus. Cheap to clone.
#[derive(Clone, Debug)]
pub struct PgEventBus {
    engine: PgEngine,
    table: String,
}

impl PgEventBus {
    /// Build a bus against the supplied [`PgEngine`]. Uses
    /// `tdw_bus` as the default table name.
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

    /// Idempotently create the bus table.
    pub async fn ensure_schema(&self) -> Result<()> {
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                sequence BIGSERIAL PRIMARY KEY, \
                envelope_json JSONB NOT NULL, \
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()\
            )",
            self.table
        );
        self.engine.execute(&sql, Value::Null).await?;
        Ok(())
    }

    /// Publish an envelope. Returns the assigned sequence (BIGSERIAL).
    pub async fn publish(&self, envelope: EventEnvelope<Value>) -> Result<u64> {
        let envelope_text = serde_json::to_string(&envelope)
            .map_err(|error| Error::Storage(format!("bus encode envelope: {error}")))?;
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
                Error::Storage("bus publish: missing sequence in INSERT RETURNING".to_string())
            })?;
        Ok(sequence as u64)
    }

    /// Read all bus entries with sequence >= `sequence`, ordered.
    pub async fn read_from(&self, sequence: u64) -> Result<Vec<BusEntry>> {
        let sql = format!(
            "SELECT sequence, envelope_json::text AS envelope_text \
             FROM {} WHERE sequence >= $1 ORDER BY sequence ASC",
            self.table
        );
        let rows = self
            .engine
            .fetch_json(&sql, json!([sequence as i64]))
            .await?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let seq = row
                .get("sequence")
                .and_then(Value::as_i64)
                .ok_or_else(|| Error::Storage("bus row: missing sequence".to_string()))?;
            let envelope_text = row
                .get("envelope_text")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::Storage("bus row: missing envelope_text".to_string()))?;
            let envelope: EventEnvelope<Value> = serde_json::from_str(envelope_text)
                .map_err(|error| Error::Storage(format!("bus decode envelope: {error}")))?;
            entries.push(BusEntry {
                sequence: seq as u64,
                envelope,
            });
        }
        Ok(entries)
    }

    /// Return the current retention window (min..max sequence).
    /// Returns `None` if the bus is empty.
    pub async fn retention_window(&self) -> Result<Option<RetentionWindow>> {
        let sql = format!(
            "SELECT MIN(sequence) AS oldest, MAX(sequence) AS newest FROM {}",
            self.table
        );
        let rows = self.engine.fetch_json(&sql, Value::Null).await?;
        let row = match rows.first() {
            Some(row) => row,
            None => return Ok(None),
        };
        let oldest = row.get("oldest").and_then(Value::as_i64);
        let newest = row.get("newest").and_then(Value::as_i64);
        match (oldest, newest) {
            (Some(o), Some(n)) => Ok(Some(RetentionWindow {
                oldest_sequence: o as u64,
                newest_sequence: n as u64,
            })),
            _ => Ok(None),
        }
    }
}
