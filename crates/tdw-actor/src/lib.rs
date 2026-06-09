#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
use serde::{Deserialize, Serialize};
use tdw_event::{Actor, ActorKind, Origin, TraceContext};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorContext {
    pub actor: Actor,
    pub origin: Origin,
    pub trace: TraceContext,
}

impl ActorContext {
    #[must_use]
    pub const fn new(actor: Actor, origin: Origin, trace: TraceContext) -> Self {
        Self {
            actor,
            origin,
            trace,
        }
    }

    pub fn service(entrypoint: impl Into<String>) -> Self {
        let entrypoint = entrypoint.into();
        Self::new(
            Actor {
                actor_id: "system:tdw".to_string(),
                kind: ActorKind::System,
                tenant_id: Some("default".to_string()),
            },
            Origin {
                service: "tdw-service".to_string(),
                entrypoint: entrypoint.clone(),
                host: None,
            },
            TraceContext {
                trace_id: format!("trace-{entrypoint}"),
                span_id: "root".to_string(),
                parent_span_id: None,
            },
        )
    }

    pub fn child_task(&self, task_name: impl Into<String>) -> ActorTaskContext {
        let task_name = task_name.into();
        ActorTaskContext {
            actor: self.actor.clone(),
            origin: self.origin.clone(),
            trace: self.trace.child(format!("task-{task_name}")),
            task_name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorTaskContext {
    pub actor: Actor,
    pub origin: Origin,
    pub trace: TraceContext,
    pub task_name: String,
}

pub struct OmcSpawn;

impl OmcSpawn {
    pub fn capture(context: &ActorContext, task_name: impl Into<String>) -> ActorTaskContext {
        context.child_task(task_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_hop_preserves_actor_and_sets_child_span() {
        let context = ActorContext::service("worker");
        let task = OmcSpawn::capture(&context, "fetch");

        assert_eq!(task.actor, context.actor);
        assert_eq!(task.origin.entrypoint, "worker");
        assert_eq!(task.trace.trace_id, context.trace.trace_id);
        assert_eq!(task.trace.parent_span_id, Some("root".to_string()));
    }

    #[test]
    fn explicit_context_constructor_preserves_callers_identity_plane() {
        let context = ActorContext::new(
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

        assert_eq!(context.actor.actor_id, "agent:researcher");
        assert_eq!(context.actor.tenant_id, Some("tenant-a".to_string()));
        assert_eq!(context.origin.host, Some("worker-1".to_string()));
        assert_eq!(context.trace.span_id, "span-live");
    }
}
