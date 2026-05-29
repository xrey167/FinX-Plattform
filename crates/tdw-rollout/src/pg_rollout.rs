//! Postgres-backed rollout sink (feature `postgres`).
//!
//! Mirrors [`crate::JsonlRollout`]'s append/read contract but persists replay
//! frames to a Postgres table instead of a JSONL file, so the daemon's rollout
//! survives container restarts in the `live` stack. Built on any
//! [`RelationalEngine`] (the daemon passes a `PgEngine`), keeping this crate
//! free of a direct `sqlx`/driver dependency.
//!
//! Schema (single table, default name `tdw_rollout`):
//!
//! ```sql
//! CREATE TABLE tdw_rollout (
//!     id          BIGSERIAL PRIMARY KEY,
//!     recorded_at TEXT   NOT NULL,
//!     session_id  TEXT   NOT NULL,
//!     sequence    BIGINT NOT NULL,
//!     frame_json  TEXT   NOT NULL
//! );
//! ```
//!
//! `frame_json` is the full [`ReplayFrame`] serialized as JSON; `read_all`
//! decodes it back, so the round trip is lossless and insertion order is
//! preserved by the monotonic `id`.

use std::sync::Arc;

use serde_json::{Value, json};
use tdw_core::{Error, RelationalEngine, Result};

use crate::RolloutRecord;

const DEFAULT_TABLE: &str = "tdw_rollout";

/// Postgres-backed rollout sink over a shared [`RelationalEngine`].
#[derive(Clone)]
pub struct PgRollout {
    engine: Arc<dyn RelationalEngine>,
    table: String,
}

impl PgRollout {
    /// Build a rollout sink against `engine`, defaulting to the `tdw_rollout`
    /// table.
    #[must_use]
    pub fn new(engine: Arc<dyn RelationalEngine>) -> Self {
        Self {
            engine,
            table: DEFAULT_TABLE.to_string(),
        }
    }

    /// Override the backing table name.
    #[must_use]
    pub fn with_table(mut self, table: impl Into<String>) -> Self {
        self.table = table.into();
        self
    }

    /// Idempotently create the backing table.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn ensure_schema(&self) -> Result<()> {
        let sql = format!(
            "CREATE TABLE IF NOT EXISTS {} (\
                id BIGSERIAL PRIMARY KEY, \
                recorded_at TEXT NOT NULL, \
                session_id TEXT NOT NULL, \
                sequence BIGINT NOT NULL, \
                frame_json TEXT NOT NULL\
            )",
            self.table
        );
        self.engine.execute(&sql, Value::Null).await?;
        Ok(())
    }

    /// Append a single rollout record.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the frame cannot be serialized or the insert
    /// fails.
    pub async fn append(&self, record: &RolloutRecord) -> Result<()> {
        let frame_json = serde_json::to_string(&record.frame)
            .map_err(|error| Error::Storage(format!("encode rollout frame: {error}")))?;
        let sql = format!(
            "INSERT INTO {} (recorded_at, session_id, sequence, frame_json) \
             VALUES ($1, $2, $3, $4)",
            self.table
        );
        self.engine
            .execute(
                &sql,
                json!([
                    record.recorded_at,
                    record.frame.session_id.as_str(),
                    record.frame.sequence,
                    frame_json,
                ]),
            )
            .await?;
        Ok(())
    }

    /// Read every rollout record in insertion order.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the query fails or a row cannot be decoded.
    pub async fn read_all(&self) -> Result<Vec<RolloutRecord>> {
        let sql = format!(
            "SELECT recorded_at, frame_json FROM {} ORDER BY id ASC",
            self.table
        );
        let rows = self.engine.fetch_json(&sql, Value::Null).await?;
        rows.iter().map(decode_row).collect()
    }
}

fn decode_row(row: &Value) -> Result<RolloutRecord> {
    let recorded_at = row
        .get("recorded_at")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Storage("rollout row: missing recorded_at".to_string()))?
        .to_string();
    let frame_json = row
        .get("frame_json")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Storage("rollout row: missing frame_json".to_string()))?;
    let frame = serde_json::from_str(frame_json)
        .map_err(|error| Error::Storage(format!("decode rollout frame: {error}")))?;
    Ok(RolloutRecord { recorded_at, frame })
}
