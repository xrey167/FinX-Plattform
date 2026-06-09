//! Worker-job bridge: [`FunctionJob`], [`FunctionJobHandler`], and
//! [`enqueue_invoke`].
//!
//! # Clock boundary
//!
//! [`FunctionJobHandler::handle`] calls [`std::time::SystemTime`] once to
//! obtain `now_ms` for the registry's `invoke`/`resume` methods.  This is the
//! only wall-clock access in this module; the core [`super::FunctionRegistry`]
//! remains clock-free (all other `now_ms` values are caller-supplied).
//!
//! # Envelope encoding
//!
//! [`FunctionJob`] is JSON-serialized and stored as the `arguments` field of an
//! [`tdw_protocol::Op::ToolCall`] with `tool_name = "tdw.functions"`.
//! [`FunctionJobHandler`] decodes it by matching on that tool name.

#![cfg(feature = "worker")]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId, ToolCallId};
use tdw_worker::{DurableWorkerQueue, EnqueueOutcome, JobHandler, WorkerJob, WorkerQueueError};

use crate::{FunctionError, FunctionRegistry};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Logical queue name used for all function jobs.
pub const FUNCTIONS_QUEUE: &str = "tdw-functions";

/// `tool_name` embedded in the [`Op::ToolCall`] envelope so handlers can
/// identify function jobs without inspecting the raw JSON.
pub const FUNCTIONS_TOOL_NAME: &str = "tdw.functions";

/// `session_id` used when building envelopes; identifies the functions subsystem.
const FUNCTIONS_SESSION: &str = "tdw-functions";

/// `actor_id` embedded in each envelope's `submitted_by` field.
pub(crate) const FUNCTIONS_ACTOR: &str = "tdw-functions";

// ---------------------------------------------------------------------------
// FunctionJob
// ---------------------------------------------------------------------------

/// The execution mode for a queued function job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobMode {
    /// Start a new invocation of `function_id` with `payload`.
    Invoke,
    /// Resume an existing run identified by `run_id`.
    Resume,
}

/// The payload carried inside a worker-queue job for this crate.
///
/// Serialized as JSON into [`Op::ToolCall::arguments`] so the worker queue's
/// normal `OpEnvelope` routing applies without protocol changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionJob {
    /// The registered function id to invoke or resume.
    pub function_id: String,
    /// The stable run identifier (caller-supplied or derived deterministically).
    pub run_id: String,
    /// The input payload (passed to `invoke`; ignored by `resume`).
    pub payload: Value,
    /// Whether this job starts a new run or resumes an existing one.
    pub mode: JobMode,
}

// ---------------------------------------------------------------------------
// Envelope helpers
// ---------------------------------------------------------------------------

/// Build a [`WorkerJob`] wrapping `func_job` in an [`Op::ToolCall`] envelope.
///
/// The `job_id` is the stable idempotency key; callers supply it so cron
/// wiring can use a deterministic id (`cron:<trigger_id>:<fire_slot_ms>`) while
/// event wiring uses a run-scoped id.
///
/// # Errors
///
/// Returns [`WorkerQueueError::InvalidJob`] if the [`SessionId`] cannot be
/// constructed (only fails on empty strings, which is impossible here).
pub fn build_worker_job(
    job_id: impl Into<String>,
    func_job: &FunctionJob,
) -> Result<WorkerJob, WorkerQueueError> {
    let arguments =
        serde_json::to_value(func_job).map_err(|e| WorkerQueueError::InvalidJob(e.to_string()))?;

    let session_id = SessionId::new(FUNCTIONS_SESSION)
        .map_err(|e| WorkerQueueError::InvalidJob(e.to_string()))?;

    let envelope = OpEnvelope::new(
        session_id,
        1,
        ActorRef {
            actor_id: FUNCTIONS_ACTOR.to_string(),
            kind: ActorKind::Worker,
            tenant_id: None,
        },
        Op::ToolCall {
            call_id: ToolCallId::new(func_job.run_id.clone())
                .map_err(|e| WorkerQueueError::InvalidJob(e.to_string()))?,
            tool_name: FUNCTIONS_TOOL_NAME.to_string(),
            arguments,
            permission_id: None,
        },
    );

    Ok(WorkerJob {
        job_id: job_id.into(),
        queue: FUNCTIONS_QUEUE.to_string(),
        envelope,
        max_attempts: 3,
        not_before_ms: 0,
        priority: 0,
    })
}

/// Extract a [`FunctionJob`] from a [`WorkerJob`]'s envelope.
///
/// # Errors
///
/// Returns a descriptive `String` if the envelope does not carry a
/// `tdw.functions` `ToolCall`, or if the `arguments` JSON cannot be
/// deserialized into [`FunctionJob`].
pub fn extract_function_job(job: &WorkerJob) -> Result<FunctionJob, String> {
    match &job.envelope.op {
        Op::ToolCall {
            tool_name,
            arguments,
            ..
        } if tool_name == FUNCTIONS_TOOL_NAME => serde_json::from_value(arguments.clone())
            .map_err(|e| format!("FunctionJob deserialize failed: {e}")),
        other => Err(format!(
            "expected ToolCall({FUNCTIONS_TOOL_NAME}), got {other:?}"
        )),
    }
}

// ---------------------------------------------------------------------------
// enqueue_invoke
// ---------------------------------------------------------------------------

/// Enqueue an `Invoke` job into a [`DurableWorkerQueue`].
///
/// The `job_id` is the caller-supplied idempotency key.  Duplicate ids are
/// silently deduplicated by the queue's `on conflict do nothing` semantics.
///
/// # Errors
///
/// Returns [`WorkerQueueError`] if the job is invalid or the enqueue fails.
pub fn enqueue_invoke<Q>(
    queue: &mut Q,
    function_id: impl Into<String>,
    payload: Value,
    run_id: impl Into<String>,
    job_id: impl Into<String>,
) -> Result<EnqueueOutcome, WorkerQueueError>
where
    Q: DurableWorkerQueue,
{
    let run_id = run_id.into();
    let func_job = FunctionJob {
        function_id: function_id.into(),
        run_id,
        payload,
        mode: JobMode::Invoke,
    };
    let job = build_worker_job(job_id, &func_job)?;
    queue.enqueue(job)
}

// ---------------------------------------------------------------------------
// FunctionJobHandler
// ---------------------------------------------------------------------------

/// A [`JobHandler`] that drives a [`FunctionRegistry`] from the worker queue.
///
/// On each leased job the handler:
/// 1. Extracts the [`FunctionJob`] from the [`Op::ToolCall`] envelope.
/// 2. Reads `now_ms` from the system clock (the only wall-clock call in this
///    crate — documented in the module-level note).
/// 3. Dispatches to [`FunctionRegistry::invoke`] (for [`JobMode::Invoke`]) or
///    [`FunctionRegistry::resume`] (for [`JobMode::Resume`]).
/// 4. Maps `Ok` → job success; [`FunctionError`] → `Err(String)` so the
///    worker's retry/dead-letter semantics apply.  Memoized steps make retries
///    cheap: already-completed steps are skipped on each attempt.
pub struct FunctionJobHandler {
    /// Shared registry; the handler holds an `Arc` so it can be cloned cheaply
    /// for multi-worker setups.
    registry: Arc<FunctionRegistry>,
}

impl FunctionJobHandler {
    /// Create a handler backed by `registry`.
    #[must_use]
    pub const fn new(registry: Arc<FunctionRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl JobHandler for FunctionJobHandler {
    async fn handle(&self, job: &WorkerJob) -> Result<(), String> {
        let func_job = extract_function_job(job)?;

        // Clock boundary: only place in this crate where SystemTime is read.
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);

        let result = match func_job.mode {
            JobMode::Invoke => {
                self.registry
                    .invoke(
                        &func_job.function_id,
                        func_job.payload,
                        func_job.run_id,
                        now_ms,
                    )
                    .await
            }
            JobMode::Resume => self.registry.resume(&func_job.run_id, now_ms).await,
        };

        result.map(|_| ()).map_err(|e: FunctionError| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// RoutingJobHandler
// ---------------------------------------------------------------------------

/// Routes worker jobs: a decodable [`FunctionJob`] is executed via
/// [`FunctionJobHandler`]; every other job is delegated to the inner handler.
///
/// Lets one serve loop run BOTH function jobs and the existing job types: a
/// job whose envelope decodes as a [`FunctionJob`] (a `tdw.functions`
/// [`Op::ToolCall`]) is dispatched to the registry, while any other job is
/// forwarded unchanged to `inner`.  The [`FunctionJob`] check is
/// non-destructive — [`extract_function_job`] borrows the job and clones the
/// envelope arguments, so the same `&WorkerJob` is handed to whichever handler
/// runs it.
pub struct RoutingJobHandler<H> {
    function_handler: FunctionJobHandler,
    inner: H,
}

impl<H> RoutingJobHandler<H> {
    /// Create a router backed by `registry` for function jobs and `inner` for
    /// everything else.
    #[must_use]
    pub const fn new(registry: Arc<FunctionRegistry>, inner: H) -> Self {
        Self {
            function_handler: FunctionJobHandler::new(registry),
            inner,
        }
    }
}

#[async_trait::async_trait]
impl<H: JobHandler> JobHandler for RoutingJobHandler<H> {
    async fn handle(&self, job: &WorkerJob) -> Result<(), String> {
        if extract_function_job(job).is_ok() {
            self.function_handler.handle(job).await
        } else {
            self.inner.handle(job).await
        }
    }
}
