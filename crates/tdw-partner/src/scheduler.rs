//! The proactive schedule seam — the daily brief job spec (the partner design
//! §2.2, partner-system W3.3).
//!
//! **Reuse, do not reinvent** (the partner design §9 DROP "a new scheduler"):
//! the scheduling spine is `tdw-cron` (`ScheduleRegistry`,
//! `spawn_cron_scheduler`, `due_triggers`, `build_job`). This crate is a pure
//! leaf, so it **cannot** take a normal dependency on `tdw-cron` — that would
//! form a cycle (`tdw-cron` → `tdw-worker` → `tdw-service-api` → `tdw-partner`).
//!
//! So the split is: this module defines the cadence + the **pure** [`BriefJobSpec`]
//! (which tool to run on the daily tick, on which queue, with which args), and
//! the daemon facade (`tdw-backend`, which legally depends on both `tdw-cron`
//! and `tdw-partner`) turns the spec into a `tdw_cron::ScheduledTrigger` and
//! registers it. The whole proactive lane is then "fan existing signals into one
//! ranked list on a cron trigger" — the ranked-list assembly is
//! [`crate::build_brief`]; the firing is `tdw-cron`; this spec is the contract
//! between them.
//!
//! The pinned-clock scheduled-fire test below still exercises the **real**
//! `tdw-cron` spine (via a dev-dependency) so the design's reuse is proven here,
//! not merely asserted.
//!
//! The event-driven path (a fired alert / fresh contradiction emitting a
//! [`crate::Nudge`] immediately off the `KnowledgeFeed` tick) stays separate
//! from this scheduled path, exactly as the design splits them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The MCP tool the daily brief job runs (the partner design §2.2 / §5).
pub const BRIEF_TOOL: &str = "tdw.partner.brief";

/// The logical worker queue the brief job is enqueued onto.
pub const BRIEF_QUEUE: &str = "partner-brief";

/// The default daily cadence: 13:00 UTC every day (a 5-field cron expression
/// `min hour dom month dow`). A fixed pre-open slot; the daemon may override it.
pub const DAILY_BRIEF_CRON: &str = "0 13 * * *";

/// The pure spec the daemon turns into a `tdw_cron::ScheduledTrigger` for the
/// daily brief (the partner design §2.2, W3.3).
///
/// Carries everything the cron trigger needs without naming any `tdw-cron` type,
/// so `tdw-partner` stays a leaf: the daemon reads these fields and constructs
/// the `ScheduledTrigger` + `OpEnvelope { Op::ToolCall { tool_name, arguments } }`
/// itself. The job-id the scheduler derives from `trigger_id` is deterministic,
/// so a retried tick dedupes — the brief fires once per slot per principal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefJobSpec {
    /// The stable per-principal trigger id (`partner-brief:<session_id>`).
    pub trigger_id: String,
    /// The 5-field cron expression the trigger fires on.
    pub cron_expr: String,
    /// The worker queue to enqueue onto.
    pub queue: String,
    /// The MCP tool the worker invokes when the trigger fires.
    pub tool_name: String,
    /// The tool arguments (the principal scope the brief is built for).
    pub arguments: Value,
}

/// Build the daily-brief [`BriefJobSpec`] for `principal` on `cron_expr`
/// (the partner design §2.2, W3.3).
///
/// The spec is pure data; the daemon (`tdw-backend`) registers it on the real
/// `tdw-cron` spine. The trigger id is per-principal so each principal's brief
/// dedupes independently and the audit trail names whose brief fired. Identity
/// is carried as args but never forged — the brief tool reads the bound
/// identity, mirroring the feedback surface.
#[must_use]
pub fn daily_brief_spec(principal: &crate::Principal, cron_expr: &str) -> BriefJobSpec {
    BriefJobSpec {
        trigger_id: format!("partner-brief:{}", principal.session_id),
        cron_expr: cron_expr.to_string(),
        queue: BRIEF_QUEUE.to_string(),
        tool_name: BRIEF_TOOL.to_string(),
        arguments: serde_json::json!({
            "session_id": principal.session_id,
            "agent_id": principal.agent_id,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_cron::{
        CronSchedule, OpEnvelope, ScheduleRegistry, ScheduledTrigger, TriggerAction, build_job,
        due_triggers,
    };
    use tdw_protocol::{ActorKind, ActorRef, Op, SessionId, ToolCallId};

    fn principal() -> crate::Principal {
        crate::Principal::new("desk-a", "agent:partner")
    }

    /// Assemble the real `tdw_cron::ScheduledTrigger` from the pure spec — this
    /// is exactly what the daemon facade (`tdw-backend`) does in production, kept
    /// here so the pinned-clock fire test exercises the genuine cron spine.
    fn trigger_from_spec(spec: &BriefJobSpec) -> ScheduledTrigger {
        let schedule = CronSchedule::parse(&spec.cron_expr).expect("valid cron expr");
        let session_id = SessionId::new("desk-a").expect("non-empty session");
        let call_id = ToolCallId::new("brief-call").expect("non-empty call id");
        let op = Op::ToolCall {
            call_id,
            tool_name: spec.tool_name.clone(),
            arguments: spec.arguments.clone(),
            permission_id: None,
        };
        let envelope = OpEnvelope::new(
            session_id,
            0,
            ActorRef {
                actor_id: "agent:partner".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            op,
        );
        ScheduledTrigger {
            id: spec.trigger_id.clone(),
            schedule,
            action: TriggerAction::Enqueue {
                envelope,
                queue: spec.queue.clone(),
                max_attempts: 3,
                priority: 0,
            },
        }
    }

    /// Parse an RFC 3339 string to epoch milliseconds (pinned-clock helper).
    fn ms_at(rfc3339: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(rfc3339)
            .expect("test timestamp")
            .timestamp_millis()
    }

    #[test]
    fn spec_is_named_and_targets_the_brief_tool() {
        let spec = daily_brief_spec(&principal(), DAILY_BRIEF_CRON);
        assert_eq!(spec.trigger_id, "partner-brief:desk-a");
        assert_eq!(spec.queue, BRIEF_QUEUE);
        assert_eq!(spec.tool_name, BRIEF_TOOL);
        assert_eq!(
            spec.arguments.get("agent_id").and_then(Value::as_str),
            Some("agent:partner")
        );
    }

    // ── W3.3: pinned-clock scheduled-fire test (real tdw-cron spine) ─────────

    #[test]
    fn daily_brief_fires_in_the_pinned_window() {
        // Register the daily 13:00 UTC brief and drive the PURE due-trigger
        // detection across a pinned window straddling 13:00 — no wall clock, no
        // sleep. The trigger must be due exactly once, at the 13:00 slot, and
        // build a deterministic worker job for the brief.
        let spec = daily_brief_spec(&principal(), DAILY_BRIEF_CRON);
        let mut registry = ScheduleRegistry::new();
        registry.add(trigger_from_spec(&spec));

        // Window: 2026-06-14 12:59:00 → 13:01:00 UTC contains the 13:00 slot.
        let last_ms = ms_at("2026-06-14T12:59:00Z");
        let now_ms = ms_at("2026-06-14T13:01:00Z");
        let due = due_triggers(&registry, last_ms, now_ms);
        assert_eq!(due.len(), 1, "the daily brief is due once in the window");
        let (fired_trigger, fire_slot_ms) = due[0];
        assert_eq!(fired_trigger.id, "partner-brief:desk-a");
        assert_eq!(
            fire_slot_ms,
            ms_at("2026-06-14T13:00:00Z"),
            "fires at the 13:00 slot"
        );

        // The scheduler builds a deterministic job from the slot, and it carries
        // the brief tool-call.
        let job = build_job(fired_trigger, fire_slot_ms).expect("build job");
        assert_eq!(job.queue, BRIEF_QUEUE);
        assert_eq!(
            job.job_id,
            format!("cron:partner-brief:desk-a:{fire_slot_ms}"),
            "deterministic per-slot job id"
        );
        match &job.envelope.op {
            Op::ToolCall { tool_name, .. } => assert_eq!(tool_name, BRIEF_TOOL),
            other => panic!("brief job must be a ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn daily_brief_not_due_outside_its_slot() {
        let spec = daily_brief_spec(&principal(), DAILY_BRIEF_CRON);
        let mut registry = ScheduleRegistry::new();
        registry.add(trigger_from_spec(&spec));

        // A window fully after the 13:00 slot, before the next day's slot.
        let last_ms = ms_at("2026-06-14T13:30:00Z");
        let now_ms = ms_at("2026-06-14T14:00:00Z");
        let due = due_triggers(&registry, last_ms, now_ms);
        assert!(due.is_empty(), "no fire outside the daily slot");
    }
}
