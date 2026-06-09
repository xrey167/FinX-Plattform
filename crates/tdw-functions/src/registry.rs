//! [`FunctionDef`], [`StepFn`], and [`FunctionRegistry`].

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::step::{InMemoryStepStore, StepContext, StepStore};
use crate::store::{InMemoryRunStore, RunRecord, RunStatus, RunStore};
use crate::{FunctionError, Trigger};

// ---------------------------------------------------------------------------
// StepFn
// ---------------------------------------------------------------------------

/// The async handler for a named function.
///
/// Implement this trait to define a function body.  The blanket
/// [`FnStepAdapter`] impl allows passing a plain closure via
/// [`FunctionDef::from_fn`] without implementing the trait directly.
///
/// ```rust,ignore
/// use tdw_functions::{FunctionDef, StepContext, Trigger};
/// use serde_json::{Value, json};
///
/// let def = FunctionDef::from_fn("my-fn", vec![], |ctx, payload| Box::pin(async move {
///     ctx.step("greet", || async { Ok(json!("hello")) }).await
/// }));
/// ```
#[async_trait]
pub trait StepFn: Send + Sync {
    /// Execute the function body.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError`] on any step or store failure.
    async fn run(&self, ctx: &StepContext, payload: Value) -> Result<Value, FunctionError>;
}

// ---------------------------------------------------------------------------
// Boxed-future type alias (used by from_fn)
// ---------------------------------------------------------------------------

/// A pinned, heap-allocated future returned by a closure-based handler.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ---------------------------------------------------------------------------
// Blanket adapter: Fn(...) -> BoxFuture implements StepFn
// ---------------------------------------------------------------------------

struct FnStepAdapter<F>(F);

impl<F> fmt::Debug for FnStepAdapter<F> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("FnStepAdapter(<fn>)")
    }
}

#[async_trait]
impl<F> StepFn for FnStepAdapter<F>
where
    F: Fn(StepContext, Value) -> BoxFuture<'static, Result<Value, FunctionError>>
        + Send
        + Sync
        + 'static,
{
    async fn run(&self, ctx: &StepContext, payload: Value) -> Result<Value, FunctionError> {
        (self.0)(ctx.clone(), payload).await
    }
}

// ---------------------------------------------------------------------------
// FunctionDef
// ---------------------------------------------------------------------------

/// A named function paired with its activation triggers and handler.
///
/// `Clone` is cheap — the handler is stored behind an `Arc`.
pub struct FunctionDef {
    /// Stable, unique identifier for the function.
    pub id: String,
    /// Zero or more triggers that may activate this function.
    pub triggers: Vec<Trigger>,
    handler: Arc<dyn StepFn>,
}

impl fmt::Debug for FunctionDef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FunctionDef")
            .field("id", &self.id)
            .field("triggers", &self.triggers)
            .field("handler", &"<dyn StepFn>")
            .finish()
    }
}

impl Clone for FunctionDef {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            triggers: self.triggers.clone(),
            handler: Arc::clone(&self.handler),
        }
    }
}

impl FunctionDef {
    /// Create a [`FunctionDef`] with an explicit [`StepFn`] implementation.
    #[must_use]
    pub fn new(id: impl Into<String>, triggers: Vec<Trigger>, handler: Arc<dyn StepFn>) -> Self {
        Self {
            id: id.into(),
            triggers,
            handler,
        }
    }

    /// Create a [`FunctionDef`] from a closure returning a [`BoxFuture`].
    ///
    /// The closure receives a **cloned** [`StepContext`] and the invocation
    /// payload.
    #[must_use]
    pub fn from_fn<F>(id: impl Into<String>, triggers: Vec<Trigger>, f: F) -> Self
    where
        F: Fn(StepContext, Value) -> BoxFuture<'static, Result<Value, FunctionError>>
            + Send
            + Sync
            + 'static,
    {
        Self::new(id, triggers, Arc::new(FnStepAdapter(f)))
    }
}

// ---------------------------------------------------------------------------
// FunctionRegistry
// ---------------------------------------------------------------------------

/// Stores registered functions and drives invocation + resumption.
///
/// `functions` is a [`BTreeMap`] so [`FunctionRegistry::enumerate`] returns
/// results in deterministic (alphabetical) id order.
pub struct FunctionRegistry {
    functions: BTreeMap<String, FunctionDef>,
    steps: Arc<dyn StepStore>,
    runs: Arc<dyn RunStore>,
}

impl fmt::Debug for FunctionRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FunctionRegistry")
            .field("functions", &self.functions.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Default for FunctionRegistry {
    fn default() -> Self {
        Self {
            functions: BTreeMap::new(),
            steps: Arc::new(InMemoryStepStore::new()),
            runs: Arc::new(InMemoryRunStore::new()),
        }
    }
}

impl FunctionRegistry {
    /// Create a new empty registry backed by in-memory stores.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry with explicit `step` and `run` store implementations.
    ///
    /// Use this to plug in Postgres-backed stores.
    #[must_use]
    pub fn with_stores(steps: Arc<dyn StepStore>, runs: Arc<dyn RunStore>) -> Self {
        Self {
            functions: BTreeMap::new(),
            steps,
            runs,
        }
    }

    /// Register a [`FunctionDef`].
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::Store`] if a function with the same `id` is
    /// already registered.
    pub fn register(&mut self, def: FunctionDef) -> Result<(), FunctionError> {
        if self.functions.contains_key(&def.id) {
            return Err(FunctionError::Store(format!(
                "function already registered: {}",
                def.id
            )));
        }
        self.functions.insert(def.id.clone(), def);
        Ok(())
    }

    /// List all registered functions as `(id, triggers)` pairs, sorted by id.
    #[must_use]
    pub fn enumerate(&self) -> Vec<(String, Vec<Trigger>)> {
        self.functions
            .values()
            .map(|def| (def.id.clone(), def.triggers.clone()))
            .collect()
    }

    /// Invoke a function by id, creating a new run record.
    ///
    /// `run_id` and `now_ms` are caller-supplied so the registry has no
    /// dependency on a clock or UUID generator.
    ///
    /// - On success, the run is marked [`RunStatus::Completed`] with the
    ///   returned value.
    /// - On error, the run is marked [`RunStatus::Failed`] and the error is
    ///   propagated.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::UnknownFunction`] if `function_id` is not
    /// registered; propagates step and store errors.
    pub async fn invoke(
        &self,
        function_id: &str,
        payload: Value,
        run_id: String,
        now_ms: i64,
    ) -> Result<Value, FunctionError> {
        let def = self
            .functions
            .get(function_id)
            .ok_or_else(|| FunctionError::UnknownFunction(function_id.to_string()))?
            .clone();

        let record = RunRecord {
            run_id: run_id.clone(),
            function_id: function_id.to_string(),
            payload: payload.clone(),
            status: RunStatus::Running,
            result: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        self.runs.insert_run(&record).await?;

        let ctx = StepContext::new(run_id.clone(), Arc::clone(&self.steps));

        match def.handler.run(&ctx, payload).await {
            Ok(value) => {
                self.runs
                    .set_status(&run_id, RunStatus::Completed, Some(value.clone()), now_ms)
                    .await?;
                Ok(value)
            }
            Err(error) => {
                self.runs
                    .set_status(&run_id, RunStatus::Failed, None, now_ms)
                    .await?;
                Err(error)
            }
        }
    }

    /// Resume a partially-completed run.
    ///
    /// Loads the existing [`RunRecord`], looks up the registered function, and
    /// re-invokes the handler with the **original** payload and the **same**
    /// `run_id`.  Already-memoized steps short-circuit, so only unfinished
    /// steps execute.
    ///
    /// # Errors
    ///
    /// Returns [`FunctionError::UnknownRun`] if `run_id` has no record;
    /// [`FunctionError::UnknownFunction`] if the function is no longer
    /// registered; propagates step and store errors.
    pub async fn resume(&self, run_id: &str, now_ms: i64) -> Result<Value, FunctionError> {
        let record = self
            .runs
            .get_run(run_id)
            .await?
            .ok_or_else(|| FunctionError::UnknownRun(run_id.to_string()))?;

        let def = self
            .functions
            .get(&record.function_id)
            .ok_or_else(|| FunctionError::UnknownFunction(record.function_id.clone()))?
            .clone();

        let ctx = StepContext::new(run_id.to_string(), Arc::clone(&self.steps));

        match def.handler.run(&ctx, record.payload).await {
            Ok(value) => {
                self.runs
                    .set_status(run_id, RunStatus::Completed, Some(value.clone()), now_ms)
                    .await?;
                Ok(value)
            }
            Err(error) => {
                self.runs
                    .set_status(run_id, RunStatus::Failed, None, now_ms)
                    .await?;
                Err(error)
            }
        }
    }
}
