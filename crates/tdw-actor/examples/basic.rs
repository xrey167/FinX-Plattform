//! Offline `tdw-actor` example: capture a child-task context from a parent and
//! show that identity is preserved while the trace span advances.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-actor --example tdw_actor_basic
//! ```

#![forbid(unsafe_code)]

use tdw_actor::{ActorContext, OmcSpawn};
use tdw_event::{Actor, ActorKind, Origin, TraceContext};

fn main() {
    // Service-initiated work: a system actor at the "worker" entrypoint.
    let context = ActorContext::service("worker");
    let task = OmcSpawn::capture(&context, "fetch");

    assert_eq!(task.actor, context.actor); // identity preserved across the hop
    assert_eq!(task.origin.entrypoint, "worker"); // origin preserved
    assert_eq!(task.trace.trace_id, context.trace.trace_id); // same trace
    assert_eq!(task.trace.parent_span_id.as_deref(), Some("root")); // child span
    assert_eq!(task.task_name, "fetch");
    println!(
        "task {:?} runs as {} on trace {} (parent span {:?})",
        task.task_name, task.actor.actor_id, task.trace.trace_id, task.trace.parent_span_id
    );

    // An explicit caller identity is preserved verbatim.
    let agent = ActorContext::new(
        Actor {
            actor_id: "agent:researcher".to_string(),
            kind: ActorKind::Agent,
            tenant_id: Some("tenant-a".to_string()),
        },
        Origin {
            service: "tdw-worker".to_string(),
            entrypoint: "agent-loop".to_string(),
            host: Some("worker-1".to_string()),
        },
        TraceContext {
            trace_id: "trace-live".to_string(),
            span_id: "span-live".to_string(),
            parent_span_id: None,
        },
    );
    let agent_task = OmcSpawn::capture(&agent, "embed");
    assert_eq!(agent_task.actor.actor_id, "agent:researcher");
    assert_eq!(agent_task.actor.tenant_id.as_deref(), Some("tenant-a"));
    assert_eq!(
        agent_task.trace.parent_span_id.as_deref(),
        Some("span-live")
    );
    println!(
        "agent task runs as {} (tenant {:?})",
        agent_task.actor.actor_id, agent_task.actor.tenant_id
    );
}
