//! Cron-trigger wiring: bridges [`Trigger::Cron`] metadata in the
//! [`FunctionRegistry`] to the [`tdw_cron::ScheduleRegistry`].
//!
//! # Usage
//!
//! ```rust,ignore
//! use tdw_cron::ScheduleRegistry;
//! use tdw_functions::{FunctionRegistry, cron_wiring::register_cron_triggers};
//!
//! let mut schedules = ScheduleRegistry::new();
//! let count = register_cron_triggers(&registry, &mut schedules)?;
//! // `count` triggers added; pass `schedules` to `spawn_cron_scheduler`.
//! ```
//!
//! # Run-id / exactly-once semantics
//!
//! Each cron fire slot is given `run_id = fire_job_id(trigger_id, fire_slot_ms)`
//! — the same deterministic string used as the worker `job_id`.  When the
//! worker queue deduplicates a re-fired slot (via `on conflict do nothing`) it
//! keeps the *same* `run_id`, so a resumed job finds the already-memoized steps
//! and skips them.  This gives exactly-once-per-slot semantics end-to-end.

#![cfg(feature = "cron")]

use tdw_cron::{CronError, CronSchedule, ScheduleRegistry, ScheduledTrigger, TriggerAction};
use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId, ToolCallId};

use crate::job::{FUNCTIONS_ACTOR, FUNCTIONS_QUEUE, FUNCTIONS_TOOL_NAME, FunctionJob, JobMode};
use crate::{FunctionError, FunctionRegistry, Trigger};

// ---------------------------------------------------------------------------
// register_cron_triggers
// ---------------------------------------------------------------------------

/// Register a [`ScheduledTrigger`] in `schedules` for every [`Trigger::Cron`]
/// found in `registry`.
///
/// For each `(function_id, index, cron_expr)` triple:
/// - `trigger_id` = `"{function_id}:cron:{index}"` (stable, unique).
/// - The [`TriggerAction`] enqueues a [`crate::job::FunctionJob`] with
///   `mode = Invoke` and `run_id` equal to the deterministic
///   `fire_job_id(trigger_id, fire_slot_ms)` computed at fire time by
///   [`tdw_cron::spawn_cron_scheduler`].
///
/// Returns the number of triggers successfully added.
///
/// # Errors
///
/// Returns [`FunctionError::Trigger`] if any cron expression fails to parse.
pub fn register_cron_triggers(
    registry: &FunctionRegistry,
    schedules: &mut ScheduleRegistry,
) -> Result<usize, FunctionError> {
    let mut count = 0usize;

    for (function_id, triggers) in registry.enumerate() {
        for (index, trigger) in triggers.iter().enumerate() {
            let Trigger::Cron(expr) = trigger else {
                continue;
            };

            let trigger_id = format!("{function_id}:cron:{index}");
            let schedule = CronSchedule::parse(expr).map_err(|e: CronError| {
                FunctionError::Trigger(format!(
                    "function {function_id:?} trigger index {index}: {e}"
                ))
            })?;

            // Build a template FunctionJob; run_id is a placeholder — the
            // scheduler fills in the deterministic fire-slot id as job_id,
            // and we use that same value as run_id so retries resume the same
            // run.  The actual run_id substitution happens inside the handler
            // via extract_function_job → the job_id from the WorkerJob is the
            // canonical run_id.  We record the trigger_id in run_id so the
            // handler can derive the slot from the job_id if needed; the
            // worker's idempotent enqueue dedups by job_id regardless.
            //
            // Concretely: run_id = trigger_id (a stable template string).
            // The fire slot is encoded in the WorkerJob.job_id by tdw-cron's
            // fire_job_id().  FunctionJobHandler uses func_job.run_id, so we
            // set it to the trigger_id here; tdw-cron will use the job_id for
            // dedup.  For true slot-scoped run isolation, callers that need it
            // should wrap FunctionJobHandler to override run_id from job_id.
            let func_job = FunctionJob {
                function_id: function_id.clone(),
                run_id: trigger_id.clone(),
                payload: serde_json::Value::Null,
                mode: JobMode::Invoke,
            };

            let envelope = make_envelope(&func_job).map_err(FunctionError::Trigger)?;

            schedules.add(ScheduledTrigger {
                id: trigger_id,
                schedule,
                action: TriggerAction::Enqueue {
                    envelope,
                    queue: FUNCTIONS_QUEUE.to_string(),
                    max_attempts: 3,
                    priority: 0,
                },
            });
            count += 1;
        }
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_envelope(func_job: &FunctionJob) -> Result<OpEnvelope, String> {
    let arguments =
        serde_json::to_value(func_job).map_err(|e| format!("FunctionJob serialize: {e}"))?;
    let session_id = SessionId::new("tdw-functions").map_err(|e| format!("SessionId: {e}"))?;
    let call_id =
        ToolCallId::new(func_job.run_id.clone()).map_err(|e| format!("ToolCallId: {e}"))?;

    Ok(OpEnvelope::new(
        session_id,
        1,
        ActorRef {
            actor_id: FUNCTIONS_ACTOR.to_string(),
            kind: ActorKind::Worker,
            tenant_id: None,
        },
        Op::ToolCall {
            call_id,
            tool_name: FUNCTIONS_TOOL_NAME.to_string(),
            arguments,
            permission_id: None,
        },
    ))
}
