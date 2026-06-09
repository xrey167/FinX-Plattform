//! Event-trigger wiring: [`EventRouter`] maps named events to matching
//! registered functions and enqueues one [`crate::job::FunctionJob`] per match.
//!
//! # Subscriber surface decision
//!
//! `tdw-event` is a typed-event-definition crate (schemas, `EventEnvelope<P>`,
//! `sample_*` helpers).  It exposes **no** subscriber/callback surface — there
//! is no `subscribe(event_name, handler)` API.  The event path is therefore
//! implemented as an explicit `dispatch_event` entry point: callers obtain an
//! [`EventRouter`] once at startup and call [`EventRouter::dispatch`] whenever
//! an event arrives.  If `tdw-event` gains a subscription surface in a later
//! PR, an adapter can wrap [`EventRouter::dispatch`] without changing its API.
//!
//! # Run-id convention
//!
//! `run_id = "{run_id_seed}:{function_id}"` — unique per (event delivery,
//! matched function) pair, deterministic for deduplication, and namespaced so
//! two functions triggered by the same event get independent runs.

#![cfg(feature = "worker")]

use std::collections::BTreeMap;

use serde_json::Value;
use tdw_worker::{DurableWorkerQueue, EnqueueOutcome, WorkerQueueError};

use crate::job::{FunctionJob, JobMode, build_worker_job};
use crate::{FunctionRegistry, Trigger};

// ---------------------------------------------------------------------------
// EventRouter
// ---------------------------------------------------------------------------

/// Maps event names to the function ids that subscribe to them.
///
/// Built from a [`FunctionRegistry`] snapshot; rebuild after registering new
/// functions.
#[derive(Clone, Debug, Default)]
pub struct EventRouter {
    /// `event_name → [function_id, …]` in deterministic (`BTreeMap`) order.
    routes: BTreeMap<String, Vec<String>>,
}

impl EventRouter {
    /// Build an [`EventRouter`] from the current state of `registry`.
    ///
    /// Only [`Trigger::Event`] entries are considered; [`Trigger::Cron`] is
    /// ignored.
    #[must_use]
    pub fn from_registry(registry: &FunctionRegistry) -> Self {
        let mut routes: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (function_id, triggers) in registry.enumerate() {
            for trigger in triggers {
                if let Trigger::Event(event_name) = trigger {
                    routes
                        .entry(event_name)
                        .or_default()
                        .push(function_id.clone());
                }
            }
        }
        Self { routes }
    }

    /// Return the function ids subscribed to `event_name`, in deterministic
    /// order.  Returns an empty slice when no function is registered for the
    /// event.
    #[must_use]
    pub fn subscribers(&self, event_name: &str) -> &[String] {
        self.routes
            .get(event_name)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Enqueue one [`FunctionJob::Invoke`] per function subscribed to
    /// `event_name`.
    ///
    /// `run_id_seed` is combined with each function id to produce a unique,
    /// deterministic `run_id`: `"{run_id_seed}:{function_id}"`.  The same
    /// string is used as the worker `job_id`, so the queue's idempotent-enqueue
    /// semantics deduplicate redeliveries of the same event.
    ///
    /// Returns the [`EnqueueOutcome`] for each matched function, in the same
    /// order as [`EventRouter::subscribers`].  When no function is subscribed
    /// to `event_name` the returned `Vec` is empty.
    ///
    /// # Errors
    ///
    /// Returns the first [`WorkerQueueError`] encountered if an enqueue fails.
    pub fn dispatch<Q>(
        &self,
        queue: &mut Q,
        event_name: &str,
        payload: Value,
        run_id_seed: &str,
        _now_ms: i64,
    ) -> Result<Vec<EnqueueOutcome>, WorkerQueueError>
    where
        Q: DurableWorkerQueue,
    {
        let mut outcomes = Vec::new();
        for function_id in self.subscribers(event_name) {
            let run_id = format!("{run_id_seed}:{function_id}");
            let func_job = FunctionJob {
                function_id: function_id.clone(),
                run_id: run_id.clone(),
                payload: payload.clone(),
                mode: JobMode::Invoke,
            };
            let job = build_worker_job(&run_id, &func_job)?;
            let outcome = queue.enqueue(job)?;
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }
}
