#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_event::EventEnvelope;
use tdw_protocol::{ApprovalDecision, PermissionId};
use thiserror::Error;

pub const TOOL_PROMPT_TEXT: &str = include_str!("tool_prompt.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionMode {
    InTransaction,
    PostCommit,
    Rollback,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookEvent {
    SessionStart,
    UserMessage,
    PreQueryRewrite,
    PreToolCall,
    PostToolCall,
    PreUdfRun,
    PostUdfRun,
    PreResponse,
    Stop,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandlerKind {
    Command { command: String, args: Vec<String> },
    Http { url: String },
    Mcp { server: String, tool: String },
    Prompt { prompt_path: String },
    Agent { agent_id: String, skill_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdditionalContext {
    pub uri: String,
    pub body: String,
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSpec {
    pub name: String,
    pub event: HookEvent,
    pub handler: HandlerKind,
    pub order: i32,
    pub enabled: bool,
    pub transaction_mode: TransactionMode,
    pub max_depth: u8,
    pub should_stop: bool,
    pub additional_contexts: Vec<AdditionalContext>,
}

impl HookSpec {
    pub fn new(name: impl Into<String>, order: i32, transaction_mode: TransactionMode) -> Self {
        let name = name.into();
        Self {
            event: HookEvent::Custom(name.clone()),
            handler: HandlerKind::Command {
                command: name.clone(),
                args: Vec::new(),
            },
            name,
            order,
            enabled: true,
            transaction_mode,
            max_depth: 8,
            should_stop: false,
            additional_contexts: Vec::new(),
        }
    }

    pub fn for_event(mut self, event: HookEvent) -> Self {
        self.event = event;
        self
    }

    pub fn with_handler(mut self, handler: HandlerKind) -> Self {
        self.handler = handler;
        self
    }

    pub fn should_stop(mut self) -> Self {
        self.should_stop = true;
        self
    }

    pub fn with_context(mut self, context: AdditionalContext) -> Self {
        self.additional_contexts.push(context);
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookOutcome {
    pub name: String,
    pub transaction_mode: TransactionMode,
    pub emitted_event_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookRuntimeOutcome {
    pub name: String,
    pub event: HookEvent,
    pub handler: HandlerKind,
    pub transaction_mode: TransactionMode,
    pub emitted_event_type: String,
    pub should_stop: bool,
    pub additional_contexts: Vec<AdditionalContext>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HookError {
    #[error("hook recursion guard blocked {0}")]
    RecursionGuard(String),
    #[error("hook depth exceeded at {0}")]
    DepthExceeded(String),
}

#[derive(Clone, Debug, Default)]
pub struct HookRegistry {
    hooks: Vec<HookSpec>,
    active: BTreeSet<String>,
}

impl HookRegistry {
    pub fn register(&mut self, hook: HookSpec) {
        self.hooks.push(hook);
        self.hooks.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then(left.name.cmp(&right.name))
        });
    }

    pub fn disable(&mut self, name: &str) {
        for hook in &mut self.hooks {
            if hook.name == name {
                hook.enabled = false;
            }
        }
    }

    pub fn execute(
        &mut self,
        envelope: &EventEnvelope<Value>,
    ) -> Result<Vec<HookOutcome>, HookError> {
        Ok(self
            .execute_runtime(envelope)?
            .into_iter()
            .map(|outcome| HookOutcome {
                name: outcome.name,
                transaction_mode: outcome.transaction_mode,
                emitted_event_type: outcome.emitted_event_type,
            })
            .collect())
    }

    pub fn execute_runtime(
        &mut self,
        envelope: &EventEnvelope<Value>,
    ) -> Result<Vec<HookRuntimeOutcome>, HookError> {
        let mut outcomes = Vec::new();
        for hook in self.hooks.clone().into_iter().filter(|hook| hook.enabled) {
            if envelope.depth >= hook.max_depth {
                return Err(HookError::DepthExceeded(hook.name));
            }
            if envelope.event_type == hook.name || self.active.contains(&hook.name) {
                return Err(HookError::RecursionGuard(hook.name));
            }
            self.active.insert(hook.name.clone());
            outcomes.push(HookRuntimeOutcome {
                event: hook.event.clone(),
                handler: hook.handler.clone(),
                emitted_event_type: format!("hook.{}", hook.name),
                name: hook.name.clone(),
                transaction_mode: hook.transaction_mode,
                should_stop: hook.should_stop,
                additional_contexts: hook.additional_contexts.clone(),
            });
            self.active.remove(&hook.name);
        }
        Ok(outcomes)
    }

    pub fn hook_names(&self) -> Vec<String> {
        self.hooks.iter().map(|hook| hook.name.clone()).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionEffect {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRule {
    pub permission: PermissionEffect,
    pub pattern: String,
    pub action: String,
}

impl PermissionRule {
    pub fn new(
        permission: PermissionEffect,
        pattern: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            permission,
            pattern: pattern.into(),
            action: action.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRules {
    pub default_permission: PermissionEffect,
    pub rules: Vec<PermissionRule>,
}

impl Default for PermissionRules {
    fn default() -> Self {
        Self {
            default_permission: PermissionEffect::Ask,
            rules: Vec::new(),
        }
    }
}

impl PermissionRules {
    pub fn push(&mut self, rule: PermissionRule) {
        self.rules.push(rule);
    }

    pub fn evaluate(&self, action: &str) -> PermissionEffect {
        self.rules
            .iter()
            .filter(|rule| pattern_matches(&rule.pattern, action))
            .map(|rule| rule.permission)
            .next_back()
            .unwrap_or(self.default_permission)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredApproval {
    pub permission_id: PermissionId,
    pub action: String,
    pub pattern: String,
    pub decision: Option<ApprovalDecision>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredApprovals {
    pending: BTreeMap<String, DeferredApproval>,
}

impl DeferredApprovals {
    pub fn request(
        &mut self,
        action: impl Into<String>,
        pattern: impl Into<String>,
    ) -> PermissionId {
        let next = self.pending.len() + 1;
        let permission_id = PermissionId::new(format!("permission-{next}"))
            .unwrap_or_else(|error| panic!("generated permission id should be valid: {error}"));
        let approval = DeferredApproval {
            permission_id: permission_id.clone(),
            action: action.into(),
            pattern: pattern.into(),
            decision: None,
        };
        self.pending
            .insert(permission_id.as_str().to_string(), approval);
        permission_id
    }

    pub fn resolve(
        &mut self,
        permission_id: &PermissionId,
        decision: ApprovalDecision,
    ) -> Option<DeferredApproval> {
        self.pending
            .remove(permission_id.as_str())
            .map(|mut approval| {
                approval.decision = Some(decision);
                approval
            })
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

pub fn tool_prompt_text() -> &'static str {
    TOOL_PROMPT_TEXT
}

fn pattern_matches(pattern: &str, action: &str) -> bool {
    pattern == "*"
        || pattern == action
        || pattern
            .strip_suffix('*')
            .is_some_and(|prefix| action.starts_with(prefix))
}

#[macro_export]
macro_rules! event_hook {
    ($name:expr, $order:expr, $mode:expr) => {
        $crate::HookSpec::new($name, $order, $mode)
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_event::sample_event;

    #[test]
    fn hooks_execute_in_deterministic_order_and_skip_disabled() {
        let mut registry = HookRegistry::default();
        registry.register(event_hook!("post", 20, TransactionMode::PostCommit));
        registry.register(event_hook!("sync", 10, TransactionMode::InTransaction));
        registry.register(event_hook!("off", 15, TransactionMode::Rollback).disabled());

        let outcomes = registry
            .execute(&sample_event("service"))
            .unwrap_or_else(|error| panic!("hooks should execute: {error}"));

        assert_eq!(outcomes[0].name, "sync");
        assert_eq!(outcomes[1].name, "post");
        assert_eq!(outcomes.len(), 2);
    }

    #[test]
    fn recursion_guard_rejects_self_emitting_hook() {
        let mut registry = HookRegistry::default();
        registry.register(event_hook!(
            "ingress.received",
            1,
            TransactionMode::InTransaction
        ));

        assert_eq!(
            registry.execute(&sample_event("service")),
            Err(HookError::RecursionGuard("ingress.received".to_string()))
        );
    }

    #[test]
    fn runtime_outcome_carries_handler_veto_and_context() {
        let mut registry = HookRegistry::default();
        registry.register(
            HookSpec::new("pre_tool_guard", 1, TransactionMode::InTransaction)
                .for_event(HookEvent::PreToolCall)
                .with_handler(HandlerKind::Prompt {
                    prompt_path: "crates/tdw-hooks/src/tool_prompt.txt".to_string(),
                })
                .with_context(AdditionalContext {
                    uri: "tdw://context/policy".to_string(),
                    body: "policy context".to_string(),
                    priority: 10,
                })
                .should_stop(),
        );

        let outcomes = registry
            .execute_runtime(&sample_event("service"))
            .unwrap_or_else(|error| panic!("hooks should execute: {error}"));

        assert_eq!(outcomes[0].event, HookEvent::PreToolCall);
        assert!(matches!(outcomes[0].handler, HandlerKind::Prompt { .. }));
        assert!(outcomes[0].should_stop);
        assert_eq!(
            outcomes[0].additional_contexts[0].uri,
            "tdw://context/policy"
        );
    }

    #[test]
    fn permission_rules_are_last_match_wins() {
        let mut rules = PermissionRules::default();
        rules.push(PermissionRule::new(
            PermissionEffect::Allow,
            "tdw.query.*",
            "tdw.query.run",
        ));
        rules.push(PermissionRule::new(
            PermissionEffect::Deny,
            "tdw.query.run",
            "tdw.query.run",
        ));

        assert_eq!(rules.evaluate("tdw.query.run"), PermissionEffect::Deny);
        assert_eq!(rules.evaluate("tdw.ingest.run"), PermissionEffect::Ask);
    }

    #[test]
    fn deferred_approvals_resolve_by_permission_id() {
        let mut approvals = DeferredApprovals::default();
        let permission_id = approvals.request("tdw.udf.run", "tdw.udf.*");

        assert_eq!(approvals.pending_count(), 1);
        let resolved = approvals
            .resolve(&permission_id, ApprovalDecision::AllowOnce)
            .expect("approval resolves");

        assert_eq!(resolved.permission_id, permission_id);
        assert_eq!(resolved.decision, Some(ApprovalDecision::AllowOnce));
        assert_eq!(approvals.pending_count(), 0);
    }

    #[test]
    fn prompt_text_is_sibling_asset() {
        assert!(tool_prompt_text().contains("TDW hook tool"));
    }
}
