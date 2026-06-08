//! Recurring-trigger spine over the TDW worker queue.
//!
//! # Overview
//!
//! This crate provides a cron-style scheduler that fires [`ScheduledTrigger`]s
//! on a periodic tick and enqueues [`WorkerJob`]s into any [`DurableWorkerQueue`]
//! implementation.  It is the **spine only**; daemon wiring belongs in a later
//! PR.
//!
//! # Core types
//!
//! - [`CronSchedule`] — wraps a parsed 5-field cron expression and computes the
//!   next fire time in epoch milliseconds strictly after a given instant.
//! - [`ScheduledTrigger`] — pairs a stable `id` with a [`CronSchedule`] and a
//!   [`TriggerAction`] (what to enqueue on fire).
//! - [`ScheduleRegistry`] — ordered collection of triggers; iterated
//!   deterministically by insertion order.
//! - [`spawn_cron_scheduler`] — the live tokio task: ticks every
//!   `TDW_CRON_TICK_SECS` seconds (default 30), detects triggers whose next fire
//!   slot falls in `(last_tick, now]`, and enqueues them with a deterministic
//!   job-id so duplicate enqueues are silently deduplicated by the queue.
//!
//! # Missed-slot semantics
//!
//! On start-up (or restart after a pause) the scheduler initialises
//! `last_tick_ms` to `now_ms`.  This means **missed fire slots are skipped**:
//! only future occurrences are enqueued.  This is intentional — catch-up
//! behaviour would enqueue a burst of stale jobs and is rarely desirable for
//! financial data pipelines.
//!
//! # Job-id stability
//!
//! Each enqueued job carries a deterministic id:
//! `cron:<trigger_id>:<fire_slot_ms>`.  Because the fire slot is rounded to the
//! exact computed next-fire time, retried ticks or duplicate scheduler instances
//! produce the same id and are deduplicated by the queue's `on conflict do
//! nothing` semantics (both `SQLite` and Postgres backends).

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use cron::Schedule;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub use tdw_protocol::OpEnvelope;
pub use tdw_worker::{DurableWorkerQueue, WorkerJob};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors produced by this crate.
#[derive(Debug, Error)]
pub enum CronError {
    /// A cron expression failed to parse.
    #[error("invalid cron expression {expr:?}: {source}")]
    InvalidExpression {
        /// The expression that failed.
        expr: String,
        /// The underlying parse error.
        #[source]
        source: cron::error::Error,
    },
}

// ---------------------------------------------------------------------------
// CronSchedule
// ---------------------------------------------------------------------------

/// A parsed 5-field cron expression with millisecond next-fire computation.
///
/// Internally the `cron` crate works with 7-field expressions (seconds +
/// year).  We accept 5-field (`min hour dom month dow`) input and expand it
/// by prepending `0` (fire on the exact minute) and appending `*` (any year).
#[derive(Clone, Debug)]
pub struct CronSchedule {
    /// The original expression as supplied by the caller, for display.
    expr: String,
    /// The parsed schedule (7-field form understood by the `cron` crate).
    schedule: Schedule,
}

impl CronSchedule {
    /// Parse a 5-field cron expression (`min hour dom month dow`).
    ///
    /// # Errors
    ///
    /// Returns [`CronError::InvalidExpression`] if the expression cannot be
    /// parsed by the `cron` crate after expansion to 7 fields.
    pub fn parse(expr: &str) -> Result<Self, CronError> {
        // Expand 5-field "m h dom mon dow" → 7-field "0 m h dom mon dow *"
        let seven_field = format!("0 {expr} *");
        let schedule =
            seven_field
                .parse::<Schedule>()
                .map_err(|source| CronError::InvalidExpression {
                    expr: expr.to_string(),
                    source,
                })?;
        Ok(Self {
            expr: expr.to_string(),
            schedule,
        })
    }

    /// The original 5-field expression.
    #[must_use]
    pub fn expression(&self) -> &str {
        &self.expr
    }

    /// The next fire time strictly after `now_ms` (epoch milliseconds UTC),
    /// returned as epoch milliseconds.  Returns `None` if the schedule has no
    /// future occurrences (e.g. an expression with a fixed year in the past,
    /// though that is unusual for 5-field expressions).
    #[must_use]
    pub fn next_after(&self, now_ms: i64) -> Option<i64> {
        let now_dt = ms_to_datetime(now_ms)?;
        // The `cron` iterator yields times strictly after the supplied instant.
        self.schedule
            .after(&now_dt)
            .next()
            .and_then(|dt| datetime_to_ms(&dt))
    }
}

// ---------------------------------------------------------------------------
// TriggerAction
// ---------------------------------------------------------------------------

/// What a [`ScheduledTrigger`] enqueues when it fires.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TriggerAction {
    /// Enqueue a [`WorkerJob`]-shaped payload: the `OpEnvelope` to execute,
    /// the logical queue name, the maximum number of attempts, and the
    /// scheduling priority.
    ///
    /// The `job_id` and `not_before_ms` fields of the resulting [`WorkerJob`]
    /// are filled in by the scheduler (deterministic id + exact fire slot).
    Enqueue {
        /// The `OpEnvelope` payload to execute.
        envelope: OpEnvelope,
        /// The logical queue name (e.g. `"default"`).
        queue: String,
        /// Maximum delivery attempts before dead-lettering.
        max_attempts: u32,
        /// Scheduling priority (higher leases first).
        priority: i32,
    },
}

// ---------------------------------------------------------------------------
// ScheduledTrigger
// ---------------------------------------------------------------------------

/// A named trigger: fires on `schedule` and performs `action`.
#[derive(Clone, Debug)]
pub struct ScheduledTrigger {
    /// Stable, unique identifier.  Used to derive deterministic job-ids.
    pub id: String,
    /// When to fire.
    pub schedule: CronSchedule,
    /// What to do on fire.
    pub action: TriggerAction,
}

// ---------------------------------------------------------------------------
// ScheduleRegistry
// ---------------------------------------------------------------------------

/// An ordered collection of [`ScheduledTrigger`]s.
///
/// Iteration is deterministic: triggers are yielded in insertion order.
/// Duplicate `id`s overwrite the existing entry but preserve insertion order.
#[derive(Clone, Debug, Default)]
pub struct ScheduleRegistry {
    /// Ordered ids for deterministic iteration.
    order: Vec<String>,
    /// The triggers keyed by id.
    triggers: HashMap<String, ScheduledTrigger>,
}

impl ScheduleRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add (or replace) a trigger.  Insertion order is preserved for new ids;
    /// replacements keep the original position.
    pub fn add(&mut self, trigger: ScheduledTrigger) {
        if !self.triggers.contains_key(&trigger.id) {
            self.order.push(trigger.id.clone());
        }
        self.triggers.insert(trigger.id.clone(), trigger);
    }

    /// Iterate triggers in deterministic insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &ScheduledTrigger> {
        self.order.iter().filter_map(|id| self.triggers.get(id))
    }

    /// The number of registered triggers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.triggers.len()
    }

    /// Whether the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Fire-slot detection (pure, time-injectable — enables deterministic tests)
// ---------------------------------------------------------------------------

/// Compute the set of (trigger, `fire_slot_ms`) pairs whose next fire time after
/// `last_tick_ms` falls at or before `now_ms`.
///
/// This is a pure function with no I/O; it is the testable core of the
/// scheduler loop.
///
/// The returned vec preserves the registry's deterministic iteration order.
#[must_use]
pub fn due_triggers(
    registry: &ScheduleRegistry,
    last_tick_ms: i64,
    now_ms: i64,
) -> Vec<(&ScheduledTrigger, i64)> {
    registry
        .iter()
        .filter_map(|trigger| {
            let fire_ms = trigger.schedule.next_after(last_tick_ms)?;
            if fire_ms <= now_ms {
                Some((trigger, fire_ms))
            } else {
                None
            }
        })
        .collect()
}

/// Build the deterministic job-id for a trigger fire slot.
///
/// Format: `cron:<trigger_id>:<fire_slot_ms>`.  This is stable across retries
/// and duplicate scheduler instances so the queue's idempotent enqueue
/// deduplicates without extra coordination.
#[must_use]
pub fn fire_job_id(trigger_id: &str, fire_slot_ms: i64) -> String {
    format!("cron:{trigger_id}:{fire_slot_ms}")
}

/// Build the [`WorkerJob`] for a trigger firing at `fire_slot_ms`.
///
/// `not_before_ms` is set to the exact fire slot so the job is eligible
/// for immediate leasing (the slot is already in the past by the time the
/// enqueue call returns).
///
/// # Errors
///
/// Returns an error string if the `TriggerAction` carries an invalid job
/// (e.g. `max_attempts == 0`), which the caller logs and continues.
pub fn build_job(trigger: &ScheduledTrigger, fire_slot_ms: i64) -> Result<WorkerJob, String> {
    match &trigger.action {
        TriggerAction::Enqueue {
            envelope,
            queue,
            max_attempts,
            priority,
        } => {
            if *max_attempts == 0 {
                return Err(format!("trigger {}: max_attempts must be > 0", trigger.id));
            }
            let not_before_ms = u64::try_from(fire_slot_ms.max(0))
                .map_err(|_| format!("trigger {}: fire_slot_ms overflow", trigger.id))?;
            Ok(WorkerJob {
                job_id: fire_job_id(&trigger.id, fire_slot_ms),
                queue: queue.clone(),
                envelope: envelope.clone(),
                max_attempts: *max_attempts,
                not_before_ms,
                priority: *priority,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// spawn_cron_scheduler
// ---------------------------------------------------------------------------

/// Spawn the periodic cron scheduler.
///
/// On each tick the scheduler:
/// 1. Reads `now_ms` from the wall clock.
/// 2. Calls [`due_triggers`] to find all triggers whose next fire slot after
///    `last_tick_ms` is `<= now_ms`.
/// 3. For each due trigger builds a [`WorkerJob`] via [`build_job`] and calls
///    [`DurableWorkerQueue::enqueue`].  A `DuplicateJob` outcome (idempotent
///    re-fire on the same slot) is silently ignored; other errors are logged
///    and the loop continues.
/// 4. Advances `last_tick_ms` to `now_ms`.
/// 5. Waits `tick` before the next iteration (or exits promptly on cancel).
///
/// **Missed-slot semantics**: `last_tick_ms` is initialised to `now_ms` at
/// startup.  Fire slots that fall before the first tick are therefore skipped.
/// This is the intended behaviour — catch-up enqueuing of stale slots is
/// almost never correct for financial data pipelines.
///
/// The tick duration defaults to 30 s and can be overridden by setting the
/// environment variable `TDW_CRON_TICK_SECS` to a positive integer.
///
/// # Arguments
///
/// - `registry` — the set of triggers to monitor (cloned into the task).
/// - `queue` — the durable queue to enqueue into, wrapped in an `Arc<Mutex>`.
/// - `cancel` — cooperative shutdown token; mirrors `spawn_consolidation_scheduler`.
#[must_use]
pub fn spawn_cron_scheduler<Q>(
    registry: ScheduleRegistry,
    queue: Arc<tokio::sync::Mutex<Q>>,
    cancel: CancellationToken,
) -> JoinHandle<()>
where
    Q: DurableWorkerQueue + Send + 'static,
{
    let tick = cron_tick();
    tokio::spawn(async move {
        // Initialise last_tick to now — skips all missed slots (see module doc).
        let mut last_tick_ms = wall_clock_ms();

        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }

            let now_ms = wall_clock_ms();
            let due = due_triggers(&registry, last_tick_ms, now_ms);

            if !due.is_empty() {
                let mut guard = queue.lock().await;
                for (trigger, fire_slot_ms) in due {
                    match build_job(trigger, fire_slot_ms) {
                        Err(error) => {
                            eprintln!("tdw-cron: skipping trigger {id}: {error}", id = trigger.id);
                        }
                        Ok(job) => {
                            match guard.enqueue(job) {
                                Ok(outcome) => {
                                    if outcome.inserted {
                                        eprintln!(
                                            "tdw-cron: enqueued job_id={} trigger={} slot={}",
                                            outcome.job_id, trigger.id, fire_slot_ms
                                        );
                                    }
                                    // !inserted → duplicate slot, silently ignored
                                }
                                Err(tdw_worker::WorkerQueueError::DuplicateJob(_)) => {
                                    // Idempotent: same fire slot already enqueued.
                                }
                                Err(error) => {
                                    eprintln!(
                                        "tdw-cron: enqueue error for trigger {id}: {error}",
                                        id = trigger.id
                                    );
                                }
                            }
                        }
                    }
                }
            }

            last_tick_ms = now_ms;
        }
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The scheduler tick from `TDW_CRON_TICK_SECS` (default 30 s).
///
/// A zero or unparseable value falls back to the default so the scheduler
/// never busy-spins.
#[must_use]
pub fn cron_tick() -> Duration {
    const DEFAULT_SECS: u64 = 30;
    let secs = std::env::var("TDW_CRON_TICK_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(DEFAULT_SECS);
    Duration::from_secs(secs)
}

/// Current wall clock time as epoch milliseconds.  Saturates to `i64::MAX` on
/// overflow (astronomically unlikely in practice).
#[must_use]
fn wall_clock_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Convert epoch milliseconds to a `DateTime<Utc>`.  Returns `None` if `ms`
/// is out of the `chrono` representable range.
#[must_use]
fn ms_to_datetime(ms: i64) -> Option<DateTime<Utc>> {
    let secs = ms.checked_div(1_000)?;
    let nanos = u32::try_from((ms % 1_000).abs() * 1_000_000).ok()?;
    Utc.timestamp_opt(secs, nanos).single()
}

/// Convert a `DateTime<Utc>` to epoch milliseconds.  Returns `None` on
/// overflow.
#[must_use]
const fn datetime_to_ms(dt: &DateTime<Utc>) -> Option<i64> {
    Some(dt.timestamp_millis())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};
    use tdw_worker::{EnqueueOutcome, InMemoryWorkerQueue, WorkerJobStatus, WorkerQueueError};

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn make_envelope() -> OpEnvelope {
        OpEnvelope::new(
            SessionId::new("cron-test").expect("session id"),
            1,
            ActorRef {
                actor_id: "cron-actor".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            Op::Shutdown,
        )
    }

    fn make_trigger(id: &str, expr: &str) -> ScheduledTrigger {
        ScheduledTrigger {
            id: id.to_string(),
            schedule: CronSchedule::parse(expr).expect("valid cron expression"),
            action: TriggerAction::Enqueue {
                envelope: make_envelope(),
                queue: "default".to_string(),
                max_attempts: 3,
                priority: 0,
            },
        }
    }

    // A minimal mock queue that records every enqueued job for inspection.
    struct CapturingQueue {
        inner: InMemoryWorkerQueue,
        captured: Vec<WorkerJob>,
    }

    impl CapturingQueue {
        fn new() -> Self {
            Self {
                inner: InMemoryWorkerQueue::default(),
                captured: Vec::new(),
            }
        }
    }

    impl DurableWorkerQueue for CapturingQueue {
        fn enqueue(&mut self, job: WorkerJob) -> tdw_worker::Result<EnqueueOutcome> {
            // Forward to InMemoryWorkerQueue first; only record in captured on success.
            match self.inner.enqueue(job.clone()) {
                Ok(outcome) => {
                    if outcome.inserted {
                        self.captured.push(job);
                    }
                    Ok(outcome)
                }
                err => err,
            }
        }

        fn lease_next_with_ttl(
            &mut self,
            worker_id: &str,
            ttl: u64,
        ) -> tdw_worker::Result<Option<tdw_worker::WorkerLease>> {
            self.inner.lease_next_with_ttl(worker_id, ttl)
        }

        fn complete(&mut self, job_id: &str) -> tdw_worker::Result<()> {
            self.inner.complete(job_id)
        }

        fn fail(
            &mut self,
            job_id: &str,
            error: &str,
            retry_after_ms: u64,
        ) -> tdw_worker::Result<WorkerJobStatus> {
            self.inner.fail(job_id, error, retry_after_ms)
        }

        fn reap_expired_leases(&mut self) -> tdw_worker::Result<u64> {
            self.inner.reap_expired_leases()
        }

        fn dead_letters(&self) -> Vec<tdw_worker::DeadLetterRecord> {
            self.inner.dead_letters()
        }

        fn stats(&self) -> tdw_worker::WorkerQueueStats {
            self.inner.stats()
        }
    }

    // -----------------------------------------------------------------------
    // CronSchedule: parse + next_after
    // -----------------------------------------------------------------------

    #[test]
    fn parse_every_5_min_is_ok() {
        let s = CronSchedule::parse("*/5 * * * *").expect("valid");
        assert_eq!(s.expression(), "*/5 * * * *");
    }

    #[test]
    fn parse_invalid_expression_returns_error() {
        let result = CronSchedule::parse("not a cron");
        assert!(result.is_err(), "invalid expression should fail");
        let err = result.expect_err("checked above");
        let msg = err.to_string();
        assert!(msg.contains("invalid cron expression"), "got: {msg}");
    }

    #[test]
    fn next_after_every_5_min_advances_correctly() {
        // 2026-06-07 12:00:00 UTC = 1_749_297_600_000 ms
        // "*/5 * * * *" next after 12:00:00 → 12:05:00
        let s = CronSchedule::parse("*/5 * * * *").expect("valid");
        // Use a known exact minute boundary.
        let base_ms = ms_at("2026-06-07T12:00:00Z");
        let next = s.next_after(base_ms).expect("has next");
        let expected = ms_at("2026-06-07T12:05:00Z");
        assert_eq!(next, expected, "every-5-min: next should be +5 min");
    }

    #[test]
    fn next_after_every_5_min_mid_interval() {
        // At 12:02:30 → next fire is 12:05:00
        let s = CronSchedule::parse("*/5 * * * *").expect("valid");
        let mid_ms = ms_at("2026-06-07T12:02:30Z");
        let next = s.next_after(mid_ms).expect("has next");
        let expected = ms_at("2026-06-07T12:05:00Z");
        assert_eq!(next, expected);
    }

    #[test]
    fn next_after_weekly_monday_0900() {
        // "0 9 * * MON" = 09:00 every Monday (use name to avoid DOW numbering
        // ambiguity: the `cron` crate's 7-field form uses 1=Sunday..7=Saturday,
        // so numeric "1" means Sunday, not Monday).
        // 2026-06-07 is a Sunday → next Monday is 2026-06-08.
        let s = CronSchedule::parse("0 9 * * MON").expect("valid");
        let sunday_ms = ms_at("2026-06-07T10:00:00Z");
        let next = s.next_after(sunday_ms).expect("has next");
        let expected = ms_at("2026-06-08T09:00:00Z");
        assert_eq!(next, expected, "weekly Monday 09:00");
    }

    #[test]
    fn next_after_returns_strictly_after_boundary() {
        // At exactly the fire second, next should be +5 min, not the same second.
        let s = CronSchedule::parse("*/5 * * * *").expect("valid");
        let exact_ms = ms_at("2026-06-07T12:05:00Z");
        let next = s.next_after(exact_ms).expect("has next");
        assert!(next > exact_ms, "next must be strictly after now");
        assert_eq!(next, ms_at("2026-06-07T12:10:00Z"));
    }

    // -----------------------------------------------------------------------
    // due_triggers: fire-slot detection
    // -----------------------------------------------------------------------

    #[test]
    fn due_triggers_detects_slot_in_window() {
        let mut registry = ScheduleRegistry::new();
        registry.add(make_trigger("t1", "*/5 * * * *"));

        // Window: 12:00:00 → 12:06:00
        let last_ms = ms_at("2026-06-07T12:00:00Z");
        let now_ms = ms_at("2026-06-07T12:06:00Z");

        let due = due_triggers(&registry, last_ms, now_ms);
        assert_eq!(due.len(), 1, "one slot (12:05) falls in the window");
        assert_eq!(due[0].0.id, "t1");
        assert_eq!(due[0].1, ms_at("2026-06-07T12:05:00Z"));
    }

    #[test]
    fn due_triggers_skips_slot_before_window() {
        let mut registry = ScheduleRegistry::new();
        registry.add(make_trigger("t1", "*/5 * * * *"));

        // Window starts after the 12:05 slot → nothing due.
        let last_ms = ms_at("2026-06-07T12:05:00Z");
        let now_ms = ms_at("2026-06-07T12:06:00Z");

        let due = due_triggers(&registry, last_ms, now_ms);
        assert!(due.is_empty(), "slot at last_tick boundary must not fire");
    }

    #[test]
    fn due_triggers_multiple_triggers_in_order() {
        // Two triggers both due — returned in insertion order.
        let mut registry = ScheduleRegistry::new();
        registry.add(make_trigger("hourly", "0 * * * *")); // every hour
        registry.add(make_trigger("every5", "*/5 * * * *")); // every 5 min

        // Window: 11:59:00 → 12:05:30
        let last_ms = ms_at("2026-06-07T11:59:00Z");
        let now_ms = ms_at("2026-06-07T12:05:30Z");

        let due = due_triggers(&registry, last_ms, now_ms);
        // hourly fires at 12:00, every5 fires at 12:00 and 12:05
        // every5 next after 11:59 → 12:00; hourly next after 11:59 → 12:00
        // Insertion order: hourly first, every5 second.
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].0.id, "hourly");
        assert_eq!(due[1].0.id, "every5");
    }

    #[test]
    fn due_triggers_empty_registry_returns_nothing() {
        let registry = ScheduleRegistry::new();
        let due = due_triggers(&registry, 0, i64::MAX);
        assert!(due.is_empty());
    }

    // -----------------------------------------------------------------------
    // fire_job_id: deterministic stability
    // -----------------------------------------------------------------------

    #[test]
    fn fire_job_id_is_deterministic() {
        let id1 = fire_job_id("my-trigger", 1_749_297_900_000);
        let id2 = fire_job_id("my-trigger", 1_749_297_900_000);
        assert_eq!(id1, id2);
        assert_eq!(id1, "cron:my-trigger:1749297900000");
    }

    #[test]
    fn fire_job_id_differs_by_slot() {
        let id1 = fire_job_id("t", 1_000);
        let id2 = fire_job_id("t", 2_000);
        assert_ne!(id1, id2);
    }

    #[test]
    fn fire_job_id_differs_by_trigger() {
        let id1 = fire_job_id("a", 1_000);
        let id2 = fire_job_id("b", 1_000);
        assert_ne!(id1, id2);
    }

    // -----------------------------------------------------------------------
    // build_job: WorkerJob construction
    // -----------------------------------------------------------------------

    #[test]
    fn build_job_sets_not_before_to_fire_slot() {
        let trigger = make_trigger("t1", "*/5 * * * *");
        let fire_ms = ms_at("2026-06-07T12:05:00Z");
        let job = build_job(&trigger, fire_ms).expect("build should succeed");

        assert_eq!(job.job_id, fire_job_id("t1", fire_ms));
        assert_eq!(job.not_before_ms, fire_ms as u64);
        assert_eq!(job.queue, "default");
        assert_eq!(job.max_attempts, 3);
        assert_eq!(job.priority, 0);
    }

    #[test]
    fn build_job_zero_max_attempts_errors() {
        let mut trigger = make_trigger("t1", "*/5 * * * *");
        trigger.action = TriggerAction::Enqueue {
            envelope: make_envelope(),
            queue: "default".to_string(),
            max_attempts: 0,
            priority: 0,
        };
        let result = build_job(&trigger, 1_000);
        assert!(result.is_err(), "max_attempts=0 must fail");
    }

    // -----------------------------------------------------------------------
    // Skip-missed-slot behaviour (simulated tick sequence)
    // -----------------------------------------------------------------------

    #[test]
    fn skip_missed_slots_on_first_tick() {
        // Simulates: scheduler starts at T=12:10:00, tick window is
        // (12:10:00, 12:11:00].  A "*/5 * * * *" trigger whose last slot
        // before now was 12:10:00 is NOT due (that slot == last_tick, not
        // strictly after).  The next due slot is 12:15:00 which is outside
        // the window, so nothing fires.
        let s = CronSchedule::parse("*/5 * * * *").expect("valid");
        let startup_ms = ms_at("2026-06-07T12:10:00Z"); // last_tick = now on start
        let tick_end_ms = ms_at("2026-06-07T12:11:00Z");

        // next_after(startup_ms) = 12:15:00, which is > tick_end_ms → not due.
        let next = s.next_after(startup_ms).expect("has next");
        assert!(
            next > tick_end_ms,
            "first slot after startup must be outside the first tick window"
        );
    }

    #[test]
    fn simulated_tick_sequence_fires_correct_slots() {
        // Manually drive a 3-tick sequence without sleeping.
        let mut registry = ScheduleRegistry::new();
        registry.add(make_trigger("t1", "*/5 * * * *"));

        // Start at 12:00:00, ticks at +6m, +11m, +16m.
        let t0 = ms_at("2026-06-07T12:00:00Z");
        let t1 = ms_at("2026-06-07T12:06:00Z"); // fires 12:05
        let t2 = ms_at("2026-06-07T12:11:00Z"); // fires 12:10
        let t3 = ms_at("2026-06-07T12:16:00Z"); // fires 12:15

        let due1 = due_triggers(&registry, t0, t1);
        assert_eq!(due1.len(), 1);
        assert_eq!(due1[0].1, ms_at("2026-06-07T12:05:00Z"));

        let due2 = due_triggers(&registry, t1, t2);
        assert_eq!(due2.len(), 1);
        assert_eq!(due2[0].1, ms_at("2026-06-07T12:10:00Z"));

        let due3 = due_triggers(&registry, t2, t3);
        assert_eq!(due3.len(), 1);
        assert_eq!(due3[0].1, ms_at("2026-06-07T12:15:00Z"));
    }

    // -----------------------------------------------------------------------
    // Enqueue deduplication via mock queue
    // -----------------------------------------------------------------------

    #[test]
    fn enqueue_same_slot_twice_deduplicates() {
        let mut queue = CapturingQueue::new();
        let trigger = make_trigger("t1", "*/5 * * * *");
        let fire_ms = ms_at("2026-06-07T12:05:00Z");

        let job1 = build_job(&trigger, fire_ms).expect("build");
        let job2 = build_job(&trigger, fire_ms).expect("build");

        let outcome1 = queue.enqueue(job1).expect("first enqueue succeeds");
        assert!(outcome1.inserted);

        // Second enqueue of the same job_id → DuplicateJob from InMemoryWorkerQueue.
        let result2 = queue.enqueue(job2);
        assert!(
            matches!(result2, Err(WorkerQueueError::DuplicateJob(_))),
            "duplicate slot must be rejected"
        );

        // Only one job was actually captured.
        assert_eq!(queue.captured.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Live scheduler: starts and stops cleanly
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scheduler_starts_and_stops_cleanly() {
        let registry = ScheduleRegistry::new(); // empty — nothing to enqueue
        let queue = Arc::new(Mutex::new(CapturingQueue::new()));
        let cancel = CancellationToken::new();

        let handle = spawn_cron_scheduler(registry, queue.clone(), cancel.clone());

        // Cancel immediately — the task should stop without hanging.
        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "scheduler must stop cleanly on cancel");

        // Nothing was enqueued (empty registry + immediate cancel).
        assert!(queue.lock().await.captured.is_empty());
    }

    // -----------------------------------------------------------------------
    // ScheduleRegistry ordering
    // -----------------------------------------------------------------------

    #[test]
    fn registry_iter_is_insertion_order() {
        let mut r = ScheduleRegistry::new();
        r.add(make_trigger("z", "0 0 * * *"));
        r.add(make_trigger("a", "0 0 * * *"));
        r.add(make_trigger("m", "0 0 * * *"));

        let ids: Vec<_> = r.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["z", "a", "m"]);
    }

    #[test]
    fn registry_replace_preserves_position() {
        let mut r = ScheduleRegistry::new();
        r.add(make_trigger("a", "0 0 * * *"));
        r.add(make_trigger("b", "0 0 * * *"));
        // Replace "a" — should remain first.
        r.add(make_trigger("a", "*/5 * * * *"));

        let ids: Vec<_> = r.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
        // The replacement expression took effect.
        assert_eq!(
            r.iter().next().expect("has first").schedule.expression(),
            "*/5 * * * *"
        );
    }

    // -----------------------------------------------------------------------
    // Test utility
    // -----------------------------------------------------------------------

    /// Parse an RFC 3339 string to epoch milliseconds.  Panics on bad input —
    /// only used in tests with hard-coded literals.
    fn ms_at(rfc3339: &str) -> i64 {
        rfc3339
            .parse::<DateTime<Utc>>()
            .expect("test timestamp")
            .timestamp_millis()
    }
}
