#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration tests covering EventEnvelope chain semantics, validator rejection
// paths, and schema bundle coverage.
//
// IMPORTANT: This file was authored offline. Verify with
// `cargo test --package tdw-event` before merging.

use serde_json::json;
use tdw_event::{
    Actor, ActorKind, EVENT_SCHEMA_NAMES, EventEnvelope, Origin, TraceContext, event_schema_bundle,
    sample_actor_context, sample_event,
};
use validator::Validate;

#[test]
fn child_event_chain_preserves_lineage_across_three_generations() {
    let root = sample_event("service");
    let child = root.child_event("hook.audit", json!({"depth": 1}));
    let grandchild = child.child_event("hook.cdc", json!({"depth": 2}));

    // causation_id walks one step up
    assert_eq!(child.causation_id.as_deref(), Some(root.event_id.as_str()));
    assert_eq!(
        grandchild.causation_id.as_deref(),
        Some(child.event_id.as_str())
    );

    // correlation_id is established at the first child and carried thereafter
    assert_eq!(
        child.correlation_id.as_deref(),
        Some(root.event_id.as_str())
    );
    assert_eq!(
        grandchild.correlation_id.as_deref(),
        Some(root.event_id.as_str())
    );

    // depth saturates upward
    assert_eq!(root.depth, 0);
    assert_eq!(child.depth, 1);
    assert_eq!(grandchild.depth, 2);
}

#[test]
fn depth_saturates_at_u8_max() {
    let mut event = sample_event("service");
    event.depth = u8::MAX;
    let child = event.child_event("hook.saturated", json!({}));
    assert_eq!(child.depth, u8::MAX, "saturating_add must clamp at u8::MAX");
}

#[test]
fn trace_context_child_links_parent_span_and_preserves_trace_id() {
    let parent = TraceContext {
        trace_id: "trace-xyz".to_string(),
        span_id: "span-a".to_string(),
        parent_span_id: None,
    };
    let child = parent.child("span-b");
    assert_eq!(child.trace_id, "trace-xyz");
    assert_eq!(child.span_id, "span-b");
    assert_eq!(child.parent_span_id.as_deref(), Some("span-a"));
}

#[test]
fn actor_rejects_empty_actor_id() {
    let actor = Actor {
        actor_id: String::new(),
        kind: ActorKind::User,
        tenant_id: None,
    };
    assert!(actor.validate().is_err());
}

#[test]
fn origin_rejects_empty_service() {
    let origin = Origin {
        service: String::new(),
        entrypoint: "boot".to_string(),
        host: None,
    };
    assert!(origin.validate().is_err());
}

#[test]
fn origin_rejects_empty_entrypoint() {
    let origin = Origin {
        service: "tdw-service".to_string(),
        entrypoint: String::new(),
        host: None,
    };
    assert!(origin.validate().is_err());
}

#[test]
fn trace_context_rejects_empty_trace_id() {
    let trace = TraceContext {
        trace_id: String::new(),
        span_id: "span".to_string(),
        parent_span_id: None,
    };
    assert!(trace.validate().is_err());
}

#[test]
fn sample_event_validates_out_of_the_box() {
    let envelope = sample_event("mcp");
    assert!(envelope.validate().is_ok());
}

#[test]
fn event_envelope_rejects_empty_event_type() {
    let (actor, origin, trace) = sample_actor_context("mcp");
    let mut envelope = EventEnvelope::new(
        "ingress.received",
        actor,
        origin,
        trace,
        "2026-05-22T00:00:00Z",
        json!({}),
    );
    envelope.event_type = String::new();
    assert!(envelope.validate().is_err());
}

#[test]
fn event_envelope_rejects_empty_occurred_at() {
    let (actor, origin, trace) = sample_actor_context("mcp");
    let mut envelope = EventEnvelope::new(
        "ingress.received",
        actor,
        origin,
        trace,
        "2026-05-22T00:00:00Z",
        json!({}),
    );
    envelope.occurred_at = String::new();
    assert!(envelope.validate().is_err());
}

#[test]
fn event_envelope_event_id_encodes_trace_span_and_type() {
    let envelope = sample_event("service");
    assert!(envelope.event_id.contains(&envelope.trace.trace_id));
    assert!(envelope.event_id.contains(&envelope.trace.span_id));
    assert!(envelope.event_id.contains(&envelope.event_type));
}

#[test]
fn event_schema_bundle_covers_every_named_schema() {
    let bundle = event_schema_bundle();
    assert_eq!(bundle.len(), EVENT_SCHEMA_NAMES.len());
    for name in EVENT_SCHEMA_NAMES {
        let schema = bundle
            .get(name)
            .unwrap_or_else(|| panic!("missing schema: {name}"));
        assert!(schema.is_object(), "schema {name} should be a JSON object");
    }
}

#[test]
fn sample_actor_context_is_deterministic_for_same_entrypoint() {
    let (a1, o1, t1) = sample_actor_context("mcp");
    let (a2, o2, t2) = sample_actor_context("mcp");
    assert_eq!(a1, a2);
    assert_eq!(o1, o2);
    assert_eq!(t1, t2);
}

#[test]
fn schema_version_defaults_to_one_zero_zero() {
    let envelope = sample_event("service");
    assert_eq!(envelope.schema_version, "1.0.0");
}
