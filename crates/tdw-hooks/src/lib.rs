#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    process::Command,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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

    #[must_use]
    pub fn for_event(mut self, event: HookEvent) -> Self {
        self.event = event;
        self
    }

    #[must_use]
    pub fn with_handler(mut self, handler: HandlerKind) -> Self {
        self.handler = handler;
        self
    }

    #[must_use]
    pub const fn should_stop(mut self) -> Self {
        self.should_stop = true;
        self
    }

    #[must_use]
    pub fn with_context(mut self, context: AdditionalContext) -> Self {
        self.additional_contexts.push(context);
        self
    }

    #[must_use]
    pub const fn disabled(mut self) -> Self {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookExecutionOutcome {
    pub runtime: HookRuntimeOutcome,
    pub action: String,
    pub permission: PermissionEffect,
    pub output: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookExecutionPolicy {
    pub permissions: PermissionRules,
    pub allow_handler_vetoes: bool,
}

impl Default for HookExecutionPolicy {
    fn default() -> Self {
        Self {
            permissions: PermissionRules {
                default_permission: PermissionEffect::Deny,
                rules: Vec::new(),
            },
            allow_handler_vetoes: false,
        }
    }
}

pub trait HookHandlerBackend {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn run_command(
        &mut self,
        command: &str,
        args: &[String],
        payload: Value,
    ) -> Result<Value, HookError>;

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn call_http(&mut self, url: &str, payload: Value) -> Result<Value, HookError>;

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn call_mcp(&mut self, server: &str, tool: &str, payload: Value) -> Result<Value, HookError>;

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn load_prompt(&mut self, prompt_path: &str, payload: Value) -> Result<Value, HookError>;

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn run_agent(
        &mut self,
        agent_id: &str,
        skill_id: &str,
        payload: Value,
    ) -> Result<Value, HookError>;
}

pub type McpHookHandler = fn(Value) -> std::result::Result<Value, String>;

pub struct SystemHookHandlerBackend {
    mcp_handlers: BTreeMap<(String, String), McpHookHandler>,
    http_bearer_token: Option<String>,
    http_timeout: Duration,
    max_response_bytes: usize,
}

impl SystemHookHandlerBackend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            mcp_handlers: BTreeMap::new(),
            http_bearer_token: None,
            http_timeout: Duration::from_secs(5),
            max_response_bytes: 64 * 1024,
        }
    }

    pub fn register_mcp_tool(
        &mut self,
        server: impl Into<String>,
        tool: impl Into<String>,
        handler: McpHookHandler,
    ) {
        self.mcp_handlers
            .insert((server.into(), tool.into()), handler);
    }

    #[must_use]
    pub const fn with_http_timeout(mut self, timeout: Duration) -> Self {
        self.http_timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_http_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.http_bearer_token = Some(token.into());
        self
    }

    #[must_use]
    pub const fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.max_response_bytes = max_response_bytes;
        self
    }
}

impl Default for SystemHookHandlerBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl HookHandlerBackend for SystemHookHandlerBackend {
    fn run_command(
        &mut self,
        command: &str,
        args: &[String],
        payload: Value,
    ) -> Result<Value, HookError> {
        let output = Command::new(command)
            .args(args)
            .output()
            .map_err(|error| HookError::HandlerFailed(command.to_string(), error.to_string()))?;
        Ok(json!({
            "command": command,
            "args": args,
            "success": output.status.success(),
            "status": output.status.code(),
            "stdout": capped_utf8(&output.stdout, self.max_response_bytes),
            "stderr": capped_utf8(&output.stderr, self.max_response_bytes),
            "payload": payload,
        }))
    }

    fn call_http(&mut self, url: &str, payload: Value) -> Result<Value, HookError> {
        let Some(token) = self.http_bearer_token.as_ref() else {
            return Err(HookError::HandlerFailed(
                url.to_string(),
                "http bearer token is not configured".to_string(),
            ));
        };
        let client = reqwest::blocking::Client::builder()
            .timeout(self.http_timeout)
            .build()
            .map_err(|error| HookError::HandlerFailed(url.to_string(), error.to_string()))?;
        let response = client
            .post(url)
            .bearer_auth(token)
            .json(&payload)
            .send()
            .map_err(|error| HookError::HandlerFailed(url.to_string(), error.to_string()))?;
        let status = response.status().as_u16();
        let mut reader = response.take((self.max_response_bytes + 1) as u64);
        let mut body = Vec::new();
        reader
            .read_to_end(&mut body)
            .map_err(|error| HookError::HandlerFailed(url.to_string(), error.to_string()))?;
        if body.len() > self.max_response_bytes {
            return Err(HookError::HandlerFailed(
                url.to_string(),
                "response too large".to_string(),
            ));
        }
        let text = String::from_utf8_lossy(&body).to_string();
        let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok(json!({ "status": status, "body": body }))
    }

    fn call_mcp(&mut self, server: &str, tool: &str, payload: Value) -> Result<Value, HookError> {
        let Some(handler) = self
            .mcp_handlers
            .get(&(server.to_string(), tool.to_string()))
        else {
            return Err(HookError::HandlerFailed(
                format!("{server}.{tool}"),
                "mcp tool is not registered".to_string(),
            ));
        };
        handler(payload)
            .map_err(|error| HookError::HandlerFailed(format!("{server}.{tool}"), error))
    }

    fn load_prompt(&mut self, prompt_path: &str, payload: Value) -> Result<Value, HookError> {
        let body = std::fs::read_to_string(prompt_path).map_err(|error| {
            HookError::HandlerFailed(prompt_path.to_string(), error.to_string())
        })?;
        Ok(json!({ "prompt_path": prompt_path, "body": body, "payload": payload }))
    }

    fn run_agent(
        &mut self,
        agent_id: &str,
        skill_id: &str,
        _payload: Value,
    ) -> Result<Value, HookError> {
        Err(HookError::HandlerFailed(
            format!("{agent_id}.{skill_id}"),
            "agent handler backend is not configured".to_string(),
        ))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HookError {
    #[error("hook recursion guard blocked {0}")]
    RecursionGuard(String),
    #[error("hook depth exceeded at {0}")]
    DepthExceeded(String),
    #[error("invalid hook spec {0}: {1}")]
    InvalidSpec(String, &'static str),
    #[error("hook permission denied for {0}")]
    PermissionDenied(String),
    #[error("hook permission requires approval for {0}")]
    PermissionRequiresApproval(String),
    #[error("hook veto denied for {0}")]
    VetoDenied(String),
    #[error("hook handler failed {0}: {1}")]
    HandlerFailed(String, String),
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

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
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

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn execute_runtime(
        &mut self,
        envelope: &EventEnvelope<Value>,
    ) -> Result<Vec<HookRuntimeOutcome>, HookError> {
        let mut outcomes = Vec::new();
        for hook in self.hooks.clone().into_iter().filter(|hook| hook.enabled) {
            validate_hook_spec(&hook)?;
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

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn execute_handlers(
        &mut self,
        envelope: &EventEnvelope<Value>,
        policy: &HookExecutionPolicy,
        backend: &mut impl HookHandlerBackend,
    ) -> Result<Vec<HookExecutionOutcome>, HookError> {
        let mut outcomes = Vec::new();
        for runtime in self.execute_runtime(envelope)? {
            let action = hook_action(&runtime)?;
            let permission = policy.permissions.evaluate(&action);
            match permission {
                PermissionEffect::Allow => {}
                PermissionEffect::Ask => {
                    return Err(HookError::PermissionRequiresApproval(action));
                }
                PermissionEffect::Deny => {
                    return Err(HookError::PermissionDenied(action));
                }
            }
            if runtime.should_stop && !policy.allow_handler_vetoes {
                return Err(HookError::VetoDenied(runtime.name));
            }

            let payload = hook_handler_payload(&runtime, envelope);
            let output = match &runtime.handler {
                HandlerKind::Command { command, args } => {
                    backend.run_command(command, args, payload)?
                }
                HandlerKind::Http { url } => backend.call_http(url, payload)?,
                HandlerKind::Mcp { server, tool } => backend.call_mcp(server, tool, payload)?,
                HandlerKind::Prompt { prompt_path } => backend.load_prompt(prompt_path, payload)?,
                HandlerKind::Agent { agent_id, skill_id } => {
                    backend.run_agent(agent_id, skill_id, payload)?
                }
            };
            outcomes.push(HookExecutionOutcome {
                runtime,
                action,
                permission,
                output,
            });
        }
        Ok(outcomes)
    }

    #[must_use]
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

    #[must_use]
    pub fn evaluate(&self, action: &str) -> PermissionEffect {
        if !is_action_name(action) {
            return PermissionEffect::Deny;
        }
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
    /// # Panics
    ///
    /// Panics if the internally generated permission id fails validation
    /// (should be unreachable in practice).
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

    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[must_use]
pub const fn tool_prompt_text() -> &'static str {
    TOOL_PROMPT_TEXT
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_hook_spec(hook: &HookSpec) -> Result<(), HookError> {
    if !is_action_name(&hook.name) {
        return Err(HookError::InvalidSpec(hook.name.clone(), "name"));
    }
    if hook.max_depth == 0 {
        return Err(HookError::InvalidSpec(hook.name.clone(), "max_depth"));
    }
    validate_handler(hook)?;
    for context in &hook.additional_contexts {
        if !is_context_uri(&context.uri) {
            return Err(HookError::InvalidSpec(hook.name.clone(), "context_uri"));
        }
    }
    Ok(())
}

fn validate_handler(hook: &HookSpec) -> Result<(), HookError> {
    match &hook.handler {
        HandlerKind::Command { command, args } => {
            if command.is_empty()
                || command
                    .chars()
                    .any(|character| matches!(character, '/' | '\\'))
                || command.chars().any(|character| {
                    character.is_control() || matches!(character, ';' | '&' | '|' | '`')
                })
            {
                return Err(HookError::InvalidSpec(hook.name.clone(), "command"));
            }
            if args.iter().any(|arg| {
                arg.chars()
                    .any(|character| character.is_control() || matches!(character, ';' | '|' | '`'))
            }) {
                return Err(HookError::InvalidSpec(hook.name.clone(), "command_args"));
            }
        }
        HandlerKind::Http { url } => {
            if !url.starts_with("https://") {
                return Err(HookError::InvalidSpec(hook.name.clone(), "url"));
            }
        }
        HandlerKind::Mcp { server, tool } => {
            if !is_action_name(server) || !is_action_name(tool) {
                return Err(HookError::InvalidSpec(hook.name.clone(), "mcp"));
            }
        }
        HandlerKind::Prompt { prompt_path } => {
            if prompt_path.is_empty()
                || prompt_path.contains("..")
                || prompt_path
                    .chars()
                    .any(|character| character.is_control() || character == '\0')
            {
                return Err(HookError::InvalidSpec(hook.name.clone(), "prompt_path"));
            }
        }
        HandlerKind::Agent { agent_id, skill_id } => {
            if !is_action_name(agent_id) || !is_action_name(skill_id) {
                return Err(HookError::InvalidSpec(hook.name.clone(), "agent"));
            }
        }
    }
    Ok(())
}

fn pattern_matches(pattern: &str, action: &str) -> bool {
    pattern == "*"
        || pattern == action
        || pattern
            .strip_suffix('*')
            .is_some_and(|prefix| action.starts_with(prefix))
}

fn is_action_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn is_context_uri(value: &str) -> bool {
    value.starts_with("tdw://") || value.starts_with("mcp://") || value.starts_with("https://")
}

fn hook_action(outcome: &HookRuntimeOutcome) -> Result<String, HookError> {
    let action = match &outcome.handler {
        HandlerKind::Command { command, .. } => format!("hook.command.{command}"),
        HandlerKind::Http { url } => {
            let parsed = reqwest::Url::parse(url)
                .map_err(|_| HookError::InvalidSpec(outcome.name.clone(), "url"))?;
            let host = parsed
                .host_str()
                .ok_or_else(|| HookError::InvalidSpec(outcome.name.clone(), "url"))?;
            format!("hook.http.{host}")
        }
        HandlerKind::Mcp { server, tool } => format!("hook.mcp.{server}.{tool}"),
        HandlerKind::Prompt { .. } => format!("hook.prompt.{}", outcome.name),
        HandlerKind::Agent { agent_id, skill_id } => {
            format!("hook.agent.{agent_id}.{skill_id}")
        }
    };
    if is_action_name(&action) {
        Ok(action)
    } else {
        Err(HookError::InvalidSpec(
            outcome.name.clone(),
            "handler_action",
        ))
    }
}

fn hook_handler_payload(outcome: &HookRuntimeOutcome, envelope: &EventEnvelope<Value>) -> Value {
    json!({
        "hook": {
            "name": outcome.name,
            "event": outcome.event,
            "transaction_mode": outcome.transaction_mode,
            "emitted_event_type": outcome.emitted_event_type,
            "should_stop": outcome.should_stop,
        },
        "envelope": envelope,
    })
}

fn capped_utf8(bytes: &[u8], max_bytes: usize) -> String {
    let keep = bytes.len().min(max_bytes);
    let mut text = String::from_utf8_lossy(&bytes[..keep]).to_string();
    if bytes.len() > max_bytes {
        text.push_str("...");
    }
    text
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

    fn policy_allow(pattern: &str) -> HookExecutionPolicy {
        let mut policy = HookExecutionPolicy::default();
        policy.permissions.push(PermissionRule::new(
            PermissionEffect::Allow,
            pattern,
            pattern,
        ));
        policy
    }

    fn mcp_echo(payload: Value) -> std::result::Result<Value, String> {
        Ok(json!({ "hook": payload["hook"]["name"].clone() }))
    }

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
    fn handler_execution_runs_allowed_command_without_shell() {
        let mut registry = HookRegistry::default();
        registry.register(
            HookSpec::new("rustc_version", 1, TransactionMode::InTransaction).with_handler(
                HandlerKind::Command {
                    command: "rustc".to_string(),
                    args: vec!["--version".to_string()],
                },
            ),
        );
        let mut backend = SystemHookHandlerBackend::new();

        let outcomes = registry
            .execute_handlers(
                &sample_event("service"),
                &policy_allow("hook.command.rustc"),
                &mut backend,
            )
            .unwrap_or_else(|error| panic!("allowed command hook should execute: {error}"));

        assert_eq!(outcomes[0].action, "hook.command.rustc");
        assert_eq!(outcomes[0].permission, PermissionEffect::Allow);
        assert!(outcomes[0].output["success"].as_bool().unwrap_or(false));
        assert!(
            outcomes[0].output["stdout"]
                .as_str()
                .unwrap_or_default()
                .contains("rustc")
        );
    }

    #[test]
    fn handler_execution_calls_registered_mcp_client() {
        let mut registry = HookRegistry::default();
        registry.register(
            HookSpec::new("mcp_echo", 1, TransactionMode::InTransaction).with_handler(
                HandlerKind::Mcp {
                    server: "local".to_string(),
                    tool: "echo".to_string(),
                },
            ),
        );
        let mut backend = SystemHookHandlerBackend::new();
        backend.register_mcp_tool("local", "echo", mcp_echo);

        let outcomes = registry
            .execute_handlers(
                &sample_event("service"),
                &policy_allow("hook.mcp.local.echo"),
                &mut backend,
            )
            .unwrap_or_else(|error| panic!("registered mcp hook should execute: {error}"));

        assert_eq!(outcomes[0].output["hook"], "mcp_echo");
    }

    #[test]
    fn handler_execution_denies_before_backend_call() {
        let mut registry = HookRegistry::default();
        registry.register(
            HookSpec::new("denied", 1, TransactionMode::InTransaction).with_handler(
                HandlerKind::Command {
                    command: "rustc".to_string(),
                    args: vec!["--version".to_string()],
                },
            ),
        );
        let mut backend = SystemHookHandlerBackend::new();

        assert_eq!(
            registry.execute_handlers(
                &sample_event("service"),
                &HookExecutionPolicy::default(),
                &mut backend,
            ),
            Err(HookError::PermissionDenied(
                "hook.command.rustc".to_string()
            ))
        );
    }

    #[test]
    fn http_veto_requires_explicit_policy() {
        let mut registry = HookRegistry::default();
        registry.register(
            HookSpec::new("remote_guard", 1, TransactionMode::InTransaction)
                .with_handler(HandlerKind::Http {
                    url: "https://example.com/tdw-hook".to_string(),
                })
                .should_stop(),
        );
        let mut backend = SystemHookHandlerBackend::new();

        assert_eq!(
            registry.execute_handlers(
                &sample_event("service"),
                &policy_allow("hook.http.example.com"),
                &mut backend,
            ),
            Err(HookError::VetoDenied("remote_guard".to_string()))
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
    fn runtime_rejects_unsafe_hook_specs_and_permission_actions() {
        let mut registry = HookRegistry::default();
        registry.register(
            HookSpec::new("bad", 1, TransactionMode::InTransaction).with_handler(
                HandlerKind::Command {
                    command: "../runner".to_string(),
                    args: Vec::new(),
                },
            ),
        );

        assert_eq!(
            registry.execute_runtime(&sample_event("service")),
            Err(HookError::InvalidSpec("bad".to_string(), "command"))
        );

        let rules = PermissionRules::default();
        assert_eq!(rules.evaluate("tdw.query.run\nall"), PermissionEffect::Deny);
    }

    #[test]
    fn prompt_text_is_sibling_asset() {
        assert!(tool_prompt_text().contains("TDW hook tool"));
    }
}
