//! Offline `tdw-hooks` example: register a hook, run it through a permission
//! policy, and show the deny-by-default policy blocking it.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-hooks --example tdw_hooks_basic
//! ```

#![forbid(unsafe_code)]

use serde_json::{Value, json};
use tdw_event::sample_event;
use tdw_hooks::{
    HookError, HookExecutionPolicy, HookHandlerBackend, HookRegistry, HookSpec, PermissionEffect,
    PermissionRule, PermissionRules, TransactionMode,
};

/// A tiny in-example backend: it records what it was asked to run and echoes a
/// JSON ack, instead of shelling out / making network calls.
#[derive(Default)]
struct RecordingBackend {
    commands: Vec<String>,
}

impl HookHandlerBackend for RecordingBackend {
    fn run_command(
        &mut self,
        command: &str,
        _args: &[String],
        _payload: Value,
    ) -> Result<Value, HookError> {
        self.commands.push(command.to_string());
        Ok(json!({ "ran": command }))
    }

    fn call_http(&mut self, _url: &str, _payload: Value) -> Result<Value, HookError> {
        Ok(Value::Null)
    }

    fn call_mcp(
        &mut self,
        _server: &str,
        _tool: &str,
        _payload: Value,
    ) -> Result<Value, HookError> {
        Ok(Value::Null)
    }

    fn load_prompt(&mut self, _prompt_path: &str, _payload: Value) -> Result<Value, HookError> {
        Ok(Value::Null)
    }

    fn run_agent(
        &mut self,
        _agent_id: &str,
        _skill_id: &str,
        _payload: Value,
    ) -> Result<Value, HookError> {
        Ok(Value::Null)
    }
}

fn main() {
    let mut registry = HookRegistry::default();
    // `HookSpec::new` gives this hook a `Command` handler named "audit.log",
    // so its policy action is "hook.command.audit.log".
    registry.register(HookSpec::new("audit.log", 10, TransactionMode::PostCommit));

    let envelope = sample_event("service"); // event_type = "ingress.received"

    // 1. Allow path: a policy that allows `hook.*` lets the handler run.
    let mut permissions = PermissionRules::default();
    permissions.push(PermissionRule::new(
        PermissionEffect::Allow,
        "hook.*",
        "allow-hooks",
    ));
    let allow_policy = HookExecutionPolicy {
        permissions,
        allow_handler_vetoes: false,
    };

    let mut backend = RecordingBackend::default();
    let outcomes = registry
        .execute_handlers(&envelope, &allow_policy, &mut backend)
        .expect("allowed hook runs");

    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].action, "hook.command.audit.log");
    assert_eq!(outcomes[0].permission, PermissionEffect::Allow);
    assert_eq!(backend.commands, vec!["audit.log".to_string()]);
    println!("allowed: ran action {}", outcomes[0].action);

    // 2. Deny path: the default policy denies by default, so the same hook is
    // blocked before the backend is touched.
    let mut denied_backend = RecordingBackend::default();
    let denied = registry.execute_handlers(
        &envelope,
        &HookExecutionPolicy::default(),
        &mut denied_backend,
    );
    assert!(matches!(denied, Err(HookError::PermissionDenied(_))));
    assert!(denied_backend.commands.is_empty());
    println!("denied: default policy blocked the hook ({denied:?})");
}
