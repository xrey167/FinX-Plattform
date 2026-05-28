#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-actor context construction and task hop.
// Authored offline. Verify with `cargo test --package tdw-actor`.

use tdw_actor::{ActorContext, OmcSpawn};
use tdw_event::ActorKind;

#[test]
fn service_context_uses_system_actor_with_default_tenant() {
    let context = ActorContext::service("worker");
    assert_eq!(context.actor.actor_id, "system:tdw");
    assert_eq!(context.actor.kind, ActorKind::System);
    assert_eq!(context.actor.tenant_id.as_deref(), Some("default"));
    assert_eq!(context.origin.service, "tdw-service");
    assert_eq!(context.origin.entrypoint, "worker");
    assert!(context.origin.host.is_none());
    assert_eq!(context.trace.trace_id, "trace-worker");
    assert_eq!(context.trace.span_id, "root");
    assert!(context.trace.parent_span_id.is_none());
}

#[test]
fn child_task_preserves_actor_and_origin() {
    let context = ActorContext::service("mcp");
    let task = context.child_task("fetch");
    assert_eq!(task.actor, context.actor);
    assert_eq!(task.origin, context.origin);
    assert_eq!(task.task_name, "fetch");
}

#[test]
fn child_task_derives_span_from_task_name() {
    let context = ActorContext::service("mcp");
    let task = context.child_task("fetch");
    assert_eq!(task.trace.trace_id, context.trace.trace_id);
    assert_eq!(task.trace.parent_span_id.as_deref(), Some("root"));
    assert_eq!(task.trace.span_id, "task-fetch");
}

#[test]
fn omc_spawn_capture_is_equivalent_to_child_task() {
    let context = ActorContext::service("service");
    let via_method = context.child_task("ingest");
    let via_spawn = OmcSpawn::capture(&context, "ingest");
    assert_eq!(via_method, via_spawn);
}

#[test]
fn multiple_tasks_get_distinct_span_ids() {
    let context = ActorContext::service("service");
    let t1 = context.child_task("a");
    let t2 = context.child_task("b");
    assert_ne!(t1.trace.span_id, t2.trace.span_id);
    assert_eq!(
        t1.trace.trace_id, t2.trace.trace_id,
        "tasks share the trace"
    );
}

#[test]
fn service_context_round_trips_via_serde() {
    let context = ActorContext::service("worker");
    let json = serde_json::to_string(&context).unwrap_or_else(|e| panic!("serialize: {e}"));
    let decoded: ActorContext =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(decoded, context);
}

#[test]
fn actor_task_context_round_trips_via_serde() {
    let context = ActorContext::service("worker");
    let task = context.child_task("fetch");
    let json = serde_json::to_string(&task).unwrap_or_else(|e| panic!("serialize: {e}"));
    let decoded: tdw_actor::ActorTaskContext =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(decoded, task);
}
