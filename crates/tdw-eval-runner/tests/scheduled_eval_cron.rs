//! Cron-tick integration tests for the K-L3 scheduled eval harness.
//!
//! These tests live in an integration test file (not in the lib) because they
//! need `tdw-cron` + `tdw-protocol` as dev-dependencies.  The scheduled_eval
//! module itself does NOT import tdw-cron to avoid the cycle:
//! `tdw-cron → tdw-worker → tdw-service-api → tdw-eval-runner → tdw-cron`.
//!
//! The tests verify:
//! 1. A trigger built from `ScheduledEvalConfig` fires exactly once per weekly
//!    slot (injected now, no real wall clock).
//! 2. The trigger id is stable and matches `eval_trigger_id(split_id)`.
//! 3. Re-registering the same trigger (replace semantics) does not duplicate it.

#![forbid(unsafe_code)]

use tdw_cron::{CronSchedule, ScheduleRegistry, ScheduledTrigger, TriggerAction, due_triggers};
use tdw_eval_runner::scheduled_eval::{ScheduledEvalConfig, eval_trigger_id};
use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_action() -> TriggerAction {
    TriggerAction::Enqueue {
        envelope: OpEnvelope::new(
            SessionId::new("tick-test").expect("session id"),
            1,
            ActorRef {
                actor_id: "eval-actor".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            Op::Shutdown,
        ),
        queue: "default".to_string(),
        max_attempts: 3,
        priority: 0,
    }
}

/// Register a K-L3 eval trigger into `registry` from `config`.
///
/// Returns `Some(trigger_id)` when `split_id` is set, `None` otherwise.
/// This is the pattern the daemon uses — it reads config and wires the
/// trigger; `tdw-eval-runner` only provides the pure eval logic.
fn register_eval_trigger(
    config: &ScheduledEvalConfig,
    registry: &mut ScheduleRegistry,
    action: TriggerAction,
) -> Option<String> {
    let split_id = config.split_id.as_deref().filter(|s| !s.is_empty())?;
    let id = eval_trigger_id(split_id);
    let schedule =
        CronSchedule::parse(&config.cadence).expect("config must be validated before wiring");
    registry.add(ScheduledTrigger {
        id: id.clone(),
        schedule,
        action,
    });
    Some(id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Epoch-ms helper: parse RFC 3339 to ms.
fn ms_at(rfc3339: &str) -> i64 {
    rfc3339
        .parse::<chrono::DateTime<chrono::Utc>>()
        .expect("test timestamp")
        .timestamp_millis()
}

#[test]
fn cron_tick_fires_once_per_weekly_slot_with_injected_now() {
    let cfg = ScheduledEvalConfig {
        split_id: Some("golden-split-v1".to_string()),
        cadence: "0 3 * * MON".to_string(), // Monday 03:00 UTC
        ..ScheduledEvalConfig::default()
    };

    let mut registry = ScheduleRegistry::new();
    let id = register_eval_trigger(&cfg, &mut registry, make_action()).expect("split_id is set");
    assert_eq!(id, "knowledge-self-eval:golden-split-v1");
    assert_eq!(registry.len(), 1);

    // last_tick = Sunday 2026-06-07 00:00:00 UTC
    // now       = Monday 2026-06-08 04:00:00 UTC  (past the 03:00 fire slot)
    let last_tick_ms = ms_at("2026-06-07T00:00:00Z");
    let now_ms = ms_at("2026-06-08T04:00:00Z");

    let due = due_triggers(&registry, last_tick_ms, now_ms);
    assert_eq!(due.len(), 1, "one weekly slot must fire");
    assert_eq!(
        due[0].0.id, "knowledge-self-eval:golden-split-v1",
        "trigger id matches"
    );

    // Advancing last_tick to now: no more slots in a 1-second window.
    let due2 = due_triggers(&registry, now_ms, now_ms + 1_000);
    assert!(due2.is_empty(), "slot must not fire twice in same window");
}

#[test]
fn cron_tick_does_not_fire_before_slot() {
    let cfg = ScheduledEvalConfig {
        split_id: Some("golden-split-v1".to_string()),
        cadence: "0 3 * * MON".to_string(),
        ..ScheduledEvalConfig::default()
    };

    let mut registry = ScheduleRegistry::new();
    register_eval_trigger(&cfg, &mut registry, make_action());

    // Window is entirely before Monday 03:00: Sunday → Sunday+1h.
    let last_tick_ms = ms_at("2026-06-07T00:00:00Z");
    let now_ms = ms_at("2026-06-07T01:00:00Z");

    let due = due_triggers(&registry, last_tick_ms, now_ms);
    assert!(due.is_empty(), "no slot in pre-Monday window");
}

#[test]
fn cron_tick_no_split_id_registers_no_trigger() {
    let cfg = ScheduledEvalConfig::default(); // split_id = None

    let mut registry = ScheduleRegistry::new();
    let result = register_eval_trigger(&cfg, &mut registry, make_action());
    assert!(result.is_none(), "None when no split_id configured");
    assert!(registry.is_empty(), "no trigger registered");
}

#[test]
fn cron_tick_replace_semantics_does_not_duplicate_trigger() {
    let cfg = ScheduledEvalConfig {
        split_id: Some("golden-split-v1".to_string()),
        cadence: "0 3 * * MON".to_string(),
        ..ScheduledEvalConfig::default()
    };

    let mut registry = ScheduleRegistry::new();
    register_eval_trigger(&cfg, &mut registry, make_action());
    register_eval_trigger(&cfg, &mut registry, make_action()); // same id → replace
    assert_eq!(registry.len(), 1, "replace must not duplicate");
}

#[test]
fn eval_trigger_id_is_stable_across_registrations() {
    // The id must be deterministic so due_triggers() deduplicates correctly
    // across restarts (the deterministic job-id contract).
    assert_eq!(
        eval_trigger_id("golden-split-v1"),
        "knowledge-self-eval:golden-split-v1"
    );
    // Two calls with the same split produce the same string.
    assert_eq!(eval_trigger_id("s"), eval_trigger_id("s"));
}
