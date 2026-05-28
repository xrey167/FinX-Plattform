#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-hooks against the worktree's origin/main API.
// Gaps targeted: max_depth → DepthExceeded, disable() of an existing hook,
// hook_names ordering, secondary sort by name when orders collide, empty
// registry behavior.
//
// IMPORTANT: Authored offline. Verify with
// `cargo test --package tdw-hooks` before merging.

use serde_json::json;
use tdw_event::{EventEnvelope, sample_actor_context};
use tdw_hooks::{HookError, HookRegistry, HookSpec, TransactionMode, event_hook};

#[test]
fn empty_registry_execute_returns_no_outcomes() {
    let mut registry = HookRegistry::default();
    let outcomes = registry
        .execute(&tdw_event::sample_event("service"))
        .unwrap_or_else(|error| panic!("empty execute: {error}"));
    assert!(outcomes.is_empty());
    assert!(registry.hook_names().is_empty());
}

#[test]
fn depth_at_or_above_max_depth_returns_depth_exceeded() {
    let mut registry = HookRegistry::default();
    registry.register(event_hook!("audit", 10, TransactionMode::PostCommit));

    let (actor, origin, trace) = sample_actor_context("service");
    let mut envelope = EventEnvelope::new(
        "ingress.received",
        actor,
        origin,
        trace,
        "2026-05-22T00:00:00Z",
        json!({}),
    );
    envelope.depth = 8; // matches HookSpec::new's default max_depth

    let result = registry.execute(&envelope);
    assert_eq!(result, Err(HookError::DepthExceeded("audit".to_string())));
}

#[test]
fn depth_strictly_below_max_depth_executes_normally() {
    let mut registry = HookRegistry::default();
    registry.register(event_hook!("audit", 10, TransactionMode::PostCommit));

    let (actor, origin, trace) = sample_actor_context("service");
    let mut envelope = EventEnvelope::new(
        "ingress.received",
        actor,
        origin,
        trace,
        "2026-05-22T00:00:00Z",
        json!({}),
    );
    envelope.depth = 7;

    let outcomes = registry
        .execute(&envelope)
        .unwrap_or_else(|error| panic!("depth 7 executes: {error}"));
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].name, "audit");
}

#[test]
fn disable_skips_subsequent_executions() {
    let mut registry = HookRegistry::default();
    registry.register(event_hook!("audit", 10, TransactionMode::PostCommit));
    registry.register(event_hook!("cdc", 20, TransactionMode::PostCommit));

    let outcomes = registry
        .execute(&tdw_event::sample_event("service"))
        .unwrap_or_else(|error| panic!("first execute: {error}"));
    assert_eq!(outcomes.len(), 2);

    registry.disable("audit");

    let outcomes = registry
        .execute(&tdw_event::sample_event("service"))
        .unwrap_or_else(|error| panic!("second execute: {error}"));
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].name, "cdc");
}

#[test]
fn disable_of_unknown_hook_is_a_no_op() {
    let mut registry = HookRegistry::default();
    registry.register(event_hook!("audit", 10, TransactionMode::PostCommit));
    registry.disable("ghost");

    let outcomes = registry
        .execute(&tdw_event::sample_event("service"))
        .unwrap_or_else(|error| panic!("execute: {error}"));
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].name, "audit");
}

#[test]
fn hooks_with_same_order_break_ties_alphabetically_by_name() {
    let mut registry = HookRegistry::default();
    registry.register(event_hook!("zebra", 10, TransactionMode::PostCommit));
    registry.register(event_hook!("alpha", 10, TransactionMode::PostCommit));
    registry.register(event_hook!("mango", 10, TransactionMode::PostCommit));

    assert_eq!(
        registry.hook_names(),
        vec![
            "alpha".to_string(),
            "mango".to_string(),
            "zebra".to_string()
        ]
    );

    let outcomes = registry
        .execute(&tdw_event::sample_event("service"))
        .unwrap_or_else(|error| panic!("execute: {error}"));
    let names: Vec<_> = outcomes.iter().map(|o| o.name.clone()).collect();
    assert_eq!(names, vec!["alpha", "mango", "zebra"]);
}

#[test]
fn emitted_event_type_is_prefixed_with_hook_namespace() {
    let mut registry = HookRegistry::default();
    registry.register(event_hook!("audit", 10, TransactionMode::PostCommit));

    let outcomes = registry
        .execute(&tdw_event::sample_event("service"))
        .unwrap_or_else(|error| panic!("execute: {error}"));
    assert_eq!(outcomes[0].emitted_event_type, "hook.audit");
}

#[test]
fn outcome_carries_the_hooks_transaction_mode() {
    let mut registry = HookRegistry::default();
    registry.register(event_hook!("sync", 10, TransactionMode::InTransaction));
    registry.register(event_hook!("post", 20, TransactionMode::PostCommit));
    registry.register(event_hook!("rb", 30, TransactionMode::Rollback));

    let outcomes = registry
        .execute(&tdw_event::sample_event("service"))
        .unwrap_or_else(|error| panic!("execute: {error}"));
    assert_eq!(outcomes[0].transaction_mode, TransactionMode::InTransaction);
    assert_eq!(outcomes[1].transaction_mode, TransactionMode::PostCommit);
    assert_eq!(outcomes[2].transaction_mode, TransactionMode::Rollback);
}

#[test]
fn hook_spec_builder_defaults_max_depth_to_eight_and_enabled_to_true() {
    let spec = HookSpec::new("h", 0, TransactionMode::InTransaction);
    assert_eq!(spec.max_depth, 8);
    assert!(spec.enabled);
    assert_eq!(spec.order, 0);
    assert_eq!(spec.name, "h");
}

#[test]
fn hook_spec_disabled_builder_flips_enabled_flag() {
    let spec = HookSpec::new("h", 0, TransactionMode::InTransaction).disabled();
    assert!(!spec.enabled);
}

#[test]
fn hook_error_display_renders_variant_payloads() {
    let recursion = HookError::RecursionGuard("ingress.received".to_string());
    assert!(recursion.to_string().contains("ingress.received"));

    let depth = HookError::DepthExceeded("audit".to_string());
    assert!(depth.to_string().contains("audit"));
}
