//! [`RunStore`] trait and [`InMemoryRunStore`].

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::FunctionError;

// ---------------------------------------------------------------------------
// RunStatus
// ---------------------------------------------------------------------------

/// Lifecycle state of a function run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    /// The run is executing (or was interrupted while executing).
    Running,
    /// The run finished successfully.
    Completed,
    /// The run terminated with an error.
    Failed,
}

impl RunStatus {
    /// Return the canonical database text representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "Running",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
        }
    }

    /// Parse from a database text representation.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] for unrecognised values.
    pub fn parse(s: &str) -> Result<Self, FunctionError> {
        match s {
            "Running" => Ok(Self::Running),
            "Completed" => Ok(Self::Completed),
            "Failed" => Ok(Self::Failed),
            other => Err(FunctionError::Store(format!("unknown run status: {other}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// RunRecord
// ---------------------------------------------------------------------------

/// A persisted record of a single function run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    /// Stable identifier for this run (caller-supplied).
    pub run_id: String,
    /// The id of the function being executed.
    pub function_id: String,
    /// The input payload supplied at invocation time.
    pub payload: Value,
    /// Current lifecycle state.
    pub status: RunStatus,
    /// The final result value, set when `status` transitions to
    /// [`RunStatus::Completed`].
    pub result: Option<Value>,
    /// Unix-epoch millisecond timestamp when the run was created.
    pub created_at_ms: i64,
    /// Unix-epoch millisecond timestamp of the last status change.
    pub updated_at_ms: i64,
}

// ---------------------------------------------------------------------------
// RunStore
// ---------------------------------------------------------------------------

/// Persistence interface for [`RunRecord`]s.
#[async_trait]
pub trait RunStore: Send + Sync {
    /// Persist a new run record.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the write fails.
    async fn insert_run(&self, record: &RunRecord) -> Result<(), FunctionError>;

    /// Return the run record for `run_id`, or `None` if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the read fails.
    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecord>, FunctionError>;

    /// Update the `status` (and optionally `result`) of an existing run.
    ///
    /// `updated_at_ms` must be the caller-supplied current timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the write fails.
    async fn set_status(
        &self,
        run_id: &str,
        status: RunStatus,
        result: Option<Value>,
        updated_at_ms: i64,
    ) -> Result<(), FunctionError>;
}

// ---------------------------------------------------------------------------
// InMemoryRunStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory implementation of [`RunStore`].
///
/// Always compiled (no feature flag). Suitable for unit tests and offline
/// scenarios.
#[derive(Debug, Default)]
pub struct InMemoryRunStore {
    records: Mutex<BTreeMap<String, RunRecord>>,
}

impl InMemoryRunStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl RunStore for InMemoryRunStore {
    async fn insert_run(&self, record: &RunRecord) -> Result<(), FunctionError> {
        self.records
            .lock()
            .map_err(|error| FunctionError::Store(format!("mutex poisoned: {error}")))?
            .insert(record.run_id.clone(), record.clone());
        Ok(())
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecord>, FunctionError> {
        Ok(self
            .records
            .lock()
            .map_err(|error| FunctionError::Store(format!("mutex poisoned: {error}")))?
            .get(run_id)
            .cloned())
    }

    async fn set_status(
        &self,
        run_id: &str,
        status: RunStatus,
        result: Option<Value>,
        updated_at_ms: i64,
    ) -> Result<(), FunctionError> {
        let mut guard = self
            .records
            .lock()
            .map_err(|error| FunctionError::Store(format!("mutex poisoned: {error}")))?;
        if let Some(record) = guard.get_mut(run_id) {
            record.status = status;
            record.result = result;
            record.updated_at_ms = updated_at_ms;
        }
        drop(guard);
        Ok(())
    }
}
