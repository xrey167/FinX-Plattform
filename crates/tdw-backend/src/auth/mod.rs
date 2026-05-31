//! Auth / hooks / policy / events surface.
//!
//! A thin re-export hub over the access-control and event-spine crates. The
//! policy enforcement, hook execution, and event-bus *behaviour* lives on the
//! [`Backend`](crate::data::Backend) and [`AgentBackend`](crate::agent::AgentBackend)
//! facades; this module simply re-exports the typed surface those methods speak
//! so callers can name principals, policies, hook specs, and event envelopes
//! without depending on the underlying crates directly. Pure re-export — no
//! business logic.

// --- Auth: principals, policies, and the authorization check ----------------
pub use tdw_auth::{AuthPolicy, Principal, authorize};

// --- Policy enforcement: the secure-endpoint surface ------------------------
pub use tdw_service_api::{
    PolicyEnforcementConfig, ServiceEndpoint, enforce_request_path_with_backend,
    mask_json_response, secure_endpoint_response,
};

// --- Hooks: the registry, specs, handler backend, and execution policy ------
pub use tdw_hooks::{
    HandlerKind, HookEvent, HookExecutionOutcome, HookExecutionPolicy, HookHandlerBackend,
    HookRegistry, HookSpec, PermissionEffect, PermissionRule, PermissionRules,
    SystemHookHandlerBackend, TransactionMode,
};

// --- Events: the event-spine envelope and its actor/origin/trace context ----
pub use tdw_event::{Actor, ActorKind, EventEnvelope, Origin, TraceContext};

#[cfg(test)]
mod tests {
    use super::*;

    /// AUTH: a [`Principal`] is authorized for a policy whose `required_role` it
    /// holds, and rejected when the role is missing. Mirrors `tdw-auth`'s own
    /// `role_policy_allows_and_denies_protected_query_paths` fixture.
    #[test]
    fn authorize_allows_matching_role_and_denies_missing_role() {
        let policy = AuthPolicy {
            table: "analytics.gold_daily_returns".to_string(),
            required_role: "analyst".to_string(),
            row_filter: Some("tenant_id = current_tenant()".to_string()),
        };

        let analyst = Principal {
            subject: "alice".to_string(),
            roles: vec!["analyst".to_string()],
        };
        assert!(
            authorize(&analyst, &policy),
            "a principal holding the required role must be authorized"
        );

        let guest = Principal {
            subject: "bob".to_string(),
            roles: vec!["guest".to_string()],
        };
        assert!(
            !authorize(&guest, &policy),
            "a principal missing the required role must be rejected"
        );
    }
}
