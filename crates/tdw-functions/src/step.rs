//! [`StepStore`] trait, [`InMemoryStepStore`], and [`StepContext`].

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::FunctionError;

// ---------------------------------------------------------------------------
// StepStore
// ---------------------------------------------------------------------------

/// Persistence interface for memoized step results.
///
/// `put` is insert-or-ignore: the first write for a `(run_id, step_name)`
/// pair wins and subsequent `put` calls for the same key are silently ignored.
/// This provides exactly-once execution semantics when a run is resumed.
#[async_trait]
pub trait StepStore: Send + Sync {
    /// Return the memoized result for `(run_id, step)`, or `None` if the step
    /// has not yet been recorded.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the underlying read fails.
    async fn get(&self, run_id: &str, step: &str) -> Result<Option<Value>, FunctionError>;

    /// Persist `value` as the result for `(run_id, step)`.
    ///
    /// First write wins; subsequent calls for the same key are no-ops.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the underlying write fails.
    async fn put(&self, run_id: &str, step: &str, value: &Value) -> Result<(), FunctionError>;

    /// Return all recorded `(step_name, value)` pairs for `run_id`, ordered
    /// by step name.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if the underlying read fails.
    async fn list(&self, run_id: &str) -> Result<Vec<(String, Value)>, FunctionError>;
}

// ---------------------------------------------------------------------------
// InMemoryStepStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory implementation of [`StepStore`].
///
/// Always compiled (no feature flag). Suitable for unit tests and offline
/// scenarios.
#[derive(Debug, Default)]
pub struct InMemoryStepStore {
    records: Mutex<BTreeMap<(String, String), Value>>,
}

impl InMemoryStepStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl StepStore for InMemoryStepStore {
    async fn get(&self, run_id: &str, step: &str) -> Result<Option<Value>, FunctionError> {
        let guard = self
            .records
            .lock()
            .map_err(|error| FunctionError::Store(format!("mutex poisoned: {error}")))?;
        let value = guard.get(&(run_id.to_string(), step.to_string())).cloned();
        drop(guard);
        Ok(value)
    }

    async fn put(&self, run_id: &str, step: &str, value: &Value) -> Result<(), FunctionError> {
        let mut guard = self
            .records
            .lock()
            .map_err(|error| FunctionError::Store(format!("mutex poisoned: {error}")))?;
        // insert-or-ignore: only insert if the key is not yet present
        guard
            .entry((run_id.to_string(), step.to_string()))
            .or_insert_with(|| value.clone());
        drop(guard);
        Ok(())
    }

    async fn list(&self, run_id: &str) -> Result<Vec<(String, Value)>, FunctionError> {
        let guard = self
            .records
            .lock()
            .map_err(|error| FunctionError::Store(format!("mutex poisoned: {error}")))?;
        Ok(guard
            .iter()
            .filter(|((rid, _), _)| rid == run_id)
            .map(|((_, step), value)| (step.clone(), value.clone()))
            .collect())
    }
}

// ---------------------------------------------------------------------------
// StepContext
// ---------------------------------------------------------------------------

/// Execution context passed to a [`super::StepFn`] handler.
///
/// The [`StepContext::step`] method wraps a step body with exactly-once
/// memoization: on a cache hit the stored value is returned without running
/// the body; on a cache miss the body runs, the result is persisted on
/// success, and the error is propagated (not persisted) on failure.
#[derive(Clone)]
pub struct StepContext {
    /// Stable identifier for this function run.
    pub run_id: String,
    /// The step-result store shared across all steps in this run.
    pub store: Arc<dyn StepStore>,
}

impl std::fmt::Debug for StepContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StepContext")
            .field("run_id", &self.run_id)
            .field("store", &"<dyn StepStore>")
            .finish()
    }
}

impl StepContext {
    /// Create a new context for `run_id` backed by `store`.
    #[must_use]
    pub fn new(run_id: String, store: Arc<dyn StepStore>) -> Self {
        Self { run_id, store }
    }

    /// Execute `f` as a named step, memoizing successful results.
    ///
    /// - **Cache hit**: returns the stored value without calling `f`.
    /// - **Cache miss**: calls `f`; on `Ok(value)` persists `value` and
    ///   returns it; on `Err` propagates the error without persisting anything
    ///   (so the step is retried on the next resume).
    ///
    /// # Errors
    ///
    /// Propagates any error returned by `f` or by the store.
    pub async fn step<F, Fut>(&self, name: &str, f: F) -> Result<Value, FunctionError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, FunctionError>>,
    {
        if let Some(cached) = self.store.get(&self.run_id, name).await? {
            return Ok(cached);
        }

        let value = f().await?;
        self.store.put(&self.run_id, name, &value).await?;
        Ok(value)
    }
}
