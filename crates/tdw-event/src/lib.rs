#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use validator::Validate;

pub const EVENT_SCHEMA_NAMES: [&str; 5] = [
    "actor",
    "origin",
    "trace_context",
    "event_envelope",
    "event_schema_ref",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ActorKind {
    User,
    Service,
    Worker,
    Agent,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Actor {
    #[validate(length(min = 1))]
    pub actor_id: String,
    pub kind: ActorKind,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct Origin {
    #[validate(length(min = 1))]
    pub service: String,
    #[validate(length(min = 1))]
    pub entrypoint: String,
    pub host: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct TraceContext {
    #[validate(length(min = 1))]
    pub trace_id: String,
    #[validate(length(min = 1))]
    pub span_id: String,
    pub parent_span_id: Option<String>,
}

impl TraceContext {
    #[must_use]
    pub fn child(&self, span_id: impl Into<String>) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            span_id: span_id.into(),
            parent_span_id: Some(self.span_id.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EventEnvelope<P> {
    #[validate(length(min = 1))]
    pub event_id: String,
    #[validate(length(min = 1))]
    pub event_type: String,
    #[validate(length(min = 1))]
    pub schema_version: String,
    pub actor: Actor,
    pub origin: Origin,
    pub trace: TraceContext,
    #[validate(length(min = 1))]
    pub occurred_at: String,
    pub payload: P,
    pub causation_id: Option<String>,
    pub correlation_id: Option<String>,
    pub depth: u8,
}

/// Process-global monotonic counter giving each `child_event` span a unique
/// suffix, so sibling events never collide on `span_id`/`event_id`.
static CHILD_SPAN_COUNTER: AtomicU64 = AtomicU64::new(0);

impl<P> EventEnvelope<P> {
    pub fn new(
        event_type: impl Into<String>,
        actor: Actor,
        origin: Origin,
        trace: TraceContext,
        occurred_at: impl Into<String>,
        payload: P,
    ) -> Self {
        let event_type = event_type.into();
        let event_id = format!("{}:{}:{}", trace.trace_id, trace.span_id, event_type);
        Self {
            event_id,
            event_type,
            schema_version: "1.0.0".to_string(),
            actor,
            origin,
            trace,
            occurred_at: occurred_at.into(),
            payload,
            causation_id: None,
            correlation_id: None,
            depth: 0,
        }
    }

    pub fn child_event<Q>(&self, event_type: impl Into<String>, payload: Q) -> EventEnvelope<Q> {
        // Each child gets a process-unique span suffix. A bare `"{parent}-child"`
        // gave every sibling the same span_id, so two same-type children of one
        // parent produced identical event_ids (event_id = trace:span:type) —
        // collapsing them in the trace/causation graph. The monotonic counter
        // keeps sibling spans (and therefore event_ids) distinct while the
        // parent linkage is still carried by `TraceContext::child`.
        let child_span = format!(
            "{}-child-{}",
            self.trace.span_id,
            CHILD_SPAN_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let mut envelope = EventEnvelope::new(
            event_type,
            self.actor.clone(),
            self.origin.clone(),
            self.trace.child(child_span),
            self.occurred_at.clone(),
            payload,
        );
        envelope.causation_id = Some(self.event_id.clone());
        envelope.correlation_id = self
            .correlation_id
            .clone()
            .or_else(|| Some(self.event_id.clone()));
        envelope.depth = self.depth.saturating_add(1);
        envelope
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventSchemaRef {
    pub event_type: String,
    pub schema_name: String,
    pub schema_version: String,
}

#[must_use]
pub fn event_schema_bundle() -> BTreeMap<&'static str, Value> {
    BTreeMap::from([
        ("actor", schema_json::<Actor>()),
        ("origin", schema_json::<Origin>()),
        ("trace_context", schema_json::<TraceContext>()),
        ("event_envelope", schema_json::<EventEnvelope<Value>>()),
        ("event_schema_ref", schema_json::<EventSchemaRef>()),
    ])
}

#[must_use]
pub fn sample_actor_context(entrypoint: &str) -> (Actor, Origin, TraceContext) {
    (
        Actor {
            actor_id: "system:tdw".to_string(),
            kind: ActorKind::System,
            tenant_id: Some("default".to_string()),
        },
        Origin {
            service: "tdw-service".to_string(),
            entrypoint: entrypoint.to_string(),
            host: None,
        },
        TraceContext {
            trace_id: format!("trace-{entrypoint}"),
            span_id: "root".to_string(),
            parent_span_id: None,
        },
    )
}

#[must_use]
pub fn sample_event(entrypoint: &str) -> EventEnvelope<Value> {
    let (actor, origin, trace) = sample_actor_context(entrypoint);
    EventEnvelope::new(
        "ingress.received",
        actor,
        origin,
        trace,
        "2026-05-21T00:00:00Z",
        json!({ "entrypoint": entrypoint }),
    )
}

fn schema_json<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T))
        .unwrap_or_else(|error| panic!("event schema should serialize: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_envelope_preserves_actor_origin_and_trace() {
        let event = sample_event("mcp");
        assert_eq!(event.actor.actor_id, "system:tdw");
        assert_eq!(event.origin.entrypoint, "mcp");
        assert_eq!(event.trace.trace_id, "trace-mcp");
        assert!(event.validate().is_ok());
    }

    #[test]
    fn child_event_sets_causation_and_depth() {
        let event = sample_event("service");
        let child = event.child_event("hook.audit", json!({"ok": true}));

        assert_eq!(child.causation_id, Some(event.event_id));
        assert_eq!(child.depth, 1);
        assert_eq!(child.trace.parent_span_id, Some("root".to_string()));
    }

    #[test]
    fn same_type_siblings_get_distinct_span_and_event_ids() {
        let parent = sample_event("service");
        let a = parent.child_event("hook.audit", json!({"n": 1}));
        let b = parent.child_event("hook.audit", json!({"n": 2}));

        // The EV1 bug: identical span_id -> identical event_id for same-type
        // siblings, collapsing them in the trace graph. They must now differ.
        assert_ne!(
            a.trace.span_id, b.trace.span_id,
            "sibling spans must differ"
        );
        assert_ne!(a.event_id, b.event_id, "sibling event_ids must differ");
        // Parent linkage is still carried correctly on both.
        assert_eq!(a.trace.parent_span_id.as_deref(), Some("root"));
        assert_eq!(b.trace.parent_span_id.as_deref(), Some("root"));
        assert_eq!(a.causation_id.as_deref(), Some(parent.event_id.as_str()));
        assert_eq!(b.causation_id.as_deref(), Some(parent.event_id.as_str()));
        // event_id still encodes the (now-unique) span_id.
        assert!(a.event_id.contains(&a.trace.span_id));
    }

    #[test]
    fn exports_event_schema_bundle() {
        let bundle = event_schema_bundle();
        for name in EVENT_SCHEMA_NAMES {
            assert!(bundle.contains_key(name), "missing schema: {name}");
        }
    }
}
