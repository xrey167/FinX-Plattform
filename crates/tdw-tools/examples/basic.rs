//! Offline `tdw-tools` example: register a tool and run it through the
//! permission-gated orchestrator (Allow vs Ask).
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-tools --example tdw_tools_basic
//! ```

#![forbid(unsafe_code)]

use serde_json::json;
use tdw_hooks::{PermissionEffect, PermissionRule, PermissionRules};
use tdw_protocol::ToolCallId;
use tdw_tools::{ToolOrchestrator, ToolRegistry, echo_tool};

fn main() {
    // Allow path: a policy allowing `tdw.echo` runs the handler.
    let mut registry = ToolRegistry::default();
    registry.register(echo_tool()).expect("register echo tool");

    let mut permissions = PermissionRules::default();
    permissions.push(PermissionRule::new(
        PermissionEffect::Allow,
        "tdw.echo",
        "tdw.echo",
    ));
    let orchestrator = ToolOrchestrator::new(registry, permissions);

    let allowed = orchestrator
        .run(
            ToolCallId::new("call-1").expect("tool call id"),
            "tdw.echo",
            json!({ "ok": true }),
        )
        .expect("allowed tool runs");
    assert_eq!(allowed.permission, PermissionEffect::Allow);
    assert_eq!(allowed.output, Some(json!({ "ok": true })));
    println!("allowed: echo returned {:?}", allowed.output);

    // Ask path: with the default policy (Ask), the call is deferred, not run.
    let mut registry2 = ToolRegistry::default();
    registry2.register(echo_tool()).expect("register echo tool");
    let deferring = ToolOrchestrator::new(registry2, PermissionRules::default());

    let asked = deferring
        .run(
            ToolCallId::new("call-2").expect("tool call id"),
            "tdw.echo",
            json!({}),
        )
        .expect("ask defers without error");
    assert_eq!(asked.permission, PermissionEffect::Ask);
    assert!(asked.output.is_none());
    assert!(asked.deferred_permission.is_some());
    println!(
        "ask: deferred for approval as {:?}",
        asked.deferred_permission
    );
}
