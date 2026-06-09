#![forbid(unsafe_code)]

#![deny(clippy::pedantic, clippy::nursery)]
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_hooks::{PermissionEffect, PermissionRules};
use tdw_protocol::{PermissionId, ToolCallId};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ToolError>;
pub type ToolHandler = fn(Value) -> Result<Value>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ToolError {
    #[error("duplicate tool: {0}")]
    DuplicateTool(String),
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("invalid tool definition: {0}")]
    InvalidDefinition(&'static str),
    #[error("tool permission denied: {0}")]
    PermissionDenied(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permission_pattern: String,
}

#[derive(Clone)]
pub struct RegisteredTool {
    pub definition: ToolDefinition,
    handler: ToolHandler,
}

impl RegisteredTool {
    pub fn new(definition: ToolDefinition, handler: ToolHandler) -> Self {
        Self {
            definition,
            handler,
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn call(&self, input: Value) -> Result<Value> {
        (self.handler)(input)
    }
}

#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, RegisteredTool>,
}

impl ToolRegistry {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn register(&mut self, tool: RegisteredTool) -> Result<()> {
        validate_tool_definition(&tool.definition)?;
        if self.tools.contains_key(&tool.definition.name) {
            return Err(ToolError::DuplicateTool(tool.definition.name));
        }
        self.tools.insert(tool.definition.name.clone(), tool);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&RegisteredTool> {
        self.tools.get(name)
    }

    #[must_use]
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|tool| tool.definition.clone())
            .collect()
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_tool_definition(definition: &ToolDefinition) -> Result<()> {
    if !is_tool_name(&definition.name) {
        return Err(ToolError::InvalidDefinition("name"));
    }
    if definition.description.trim().is_empty() {
        return Err(ToolError::InvalidDefinition("description"));
    }
    if !is_permission_pattern(&definition.permission_pattern) {
        return Err(ToolError::InvalidDefinition("permission_pattern"));
    }
    Ok(())
}

#[derive(Clone)]
pub struct ToolRouter {
    registry: ToolRegistry,
}

impl ToolRouter {
    #[must_use]
    pub const fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn route(&self, tool_name: &str) -> Result<&RegisteredTool> {
        self.registry
            .get(tool_name)
            .ok_or_else(|| ToolError::UnknownTool(tool_name.to_string()))
    }
}

#[derive(Clone)]
pub struct ToolOrchestrator {
    router: ToolRouter,
    permissions: PermissionRules,
}

impl ToolOrchestrator {
    #[must_use]
    pub const fn new(registry: ToolRegistry, permissions: PermissionRules) -> Self {
        Self {
            router: ToolRouter::new(registry),
            permissions,
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    ///
    /// # Panics
    ///
    /// Panics if the generated deferred-permission id fails validation
    /// (should be unreachable in practice).
    pub fn run(
        &self,
        call_id: ToolCallId,
        tool_name: &str,
        arguments: Value,
    ) -> Result<OrchestratorRunResult> {
        let tool = self.router.route(tool_name)?;
        let permission = self
            .permissions
            .evaluate(&tool.definition.permission_pattern);
        match permission {
            PermissionEffect::Allow => Ok(OrchestratorRunResult {
                call_id,
                tool_name: tool.definition.name.clone(),
                permission,
                output: Some(tool.call(arguments)?),
                deferred_permission: None,
            }),
            PermissionEffect::Ask => Ok(OrchestratorRunResult {
                call_id,
                tool_name: tool.definition.name.clone(),
                permission,
                output: None,
                deferred_permission: Some(
                    PermissionId::new(format!(
                        "permission-{}",
                        tool.definition.name.replace('.', "-")
                    ))
                    .unwrap_or_else(|error| {
                        panic!("generated permission id should be valid: {error}")
                    }),
                ),
            }),
            PermissionEffect::Deny => {
                Err(ToolError::PermissionDenied(tool.definition.name.clone()))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestratorRunResult {
    pub call_id: ToolCallId,
    pub tool_name: String,
    pub permission: PermissionEffect,
    pub output: Option<Value>,
    pub deferred_permission: Option<PermissionId>,
}

pub fn echo_tool() -> RegisteredTool {
    RegisteredTool::new(
        ToolDefinition {
            name: "tdw.echo".to_string(),
            description: "Echo JSON input for contract tests.".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            permission_pattern: "tdw.echo".to_string(),
        },
        Ok,
    )
}

fn is_tool_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(is_name_character))
}

fn is_permission_pattern(value: &str) -> bool {
    value == "*"
        || value.strip_suffix('*').map_or_else(
            || is_tool_name(value),
            |prefix| prefix.ends_with('.') && is_tool_name(prefix.trim_end_matches('.')),
        )
}

const fn is_name_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tdw_hooks::{PermissionEffect, PermissionRule};

    #[test]
    fn registry_rejects_duplicate_tools() {
        let mut registry = ToolRegistry::default();
        registry
            .register(echo_tool())
            .unwrap_or_else(|error| panic!("tool should register: {error}"));

        assert_eq!(
            registry.register(echo_tool()),
            Err(ToolError::DuplicateTool("tdw.echo".to_string()))
        );
    }

    #[test]
    fn orchestrator_runs_allowed_tool() {
        let mut registry = ToolRegistry::default();
        registry
            .register(echo_tool())
            .unwrap_or_else(|error| panic!("tool should register: {error}"));
        let mut permissions = PermissionRules::default();
        permissions.push(PermissionRule::new(
            PermissionEffect::Allow,
            "tdw.echo",
            "tdw.echo",
        ));
        let orchestrator = ToolOrchestrator::new(registry, permissions);

        let result = orchestrator
            .run(
                ToolCallId::new("call-1").expect("tool call id"),
                "tdw.echo",
                json!({"ok": true}),
            )
            .unwrap_or_else(|error| panic!("tool should run: {error}"));

        assert_eq!(result.permission, PermissionEffect::Allow);
        assert_eq!(result.output, Some(json!({"ok": true})));
    }

    #[test]
    fn orchestrator_defers_ask_permission() {
        let mut registry = ToolRegistry::default();
        registry
            .register(echo_tool())
            .unwrap_or_else(|error| panic!("tool should register: {error}"));
        let orchestrator = ToolOrchestrator::new(registry, PermissionRules::default());

        let result = orchestrator
            .run(
                ToolCallId::new("call-2").expect("tool call id"),
                "tdw.echo",
                json!({}),
            )
            .unwrap_or_else(|error| panic!("ask should defer: {error}"));

        assert_eq!(result.permission, PermissionEffect::Ask);
        assert!(result.output.is_none());
        assert!(result.deferred_permission.is_some());
    }

    #[test]
    fn registry_rejects_unsafe_tool_definition() {
        let mut registry = ToolRegistry::default();
        let unsafe_tool = RegisteredTool::new(
            ToolDefinition {
                name: "../tdw.echo".to_string(),
                description: "unsafe".to_string(),
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "object"}),
                permission_pattern: "tdw.echo".to_string(),
            },
            Ok,
        );

        assert_eq!(
            registry.register(unsafe_tool),
            Err(ToolError::InvalidDefinition("name"))
        );
    }

    #[test]
    fn orchestrator_reports_unknown_and_denied_and_validates_remaining_fields() {
        // Routing an unregistered tool yields UnknownTool.
        let empty = ToolOrchestrator::new(ToolRegistry::default(), PermissionRules::default());
        assert_eq!(
            empty.run(
                ToolCallId::new("c0").expect("tool call id"),
                "tdw.missing",
                json!({})
            ),
            Err(ToolError::UnknownTool("tdw.missing".to_string()))
        );

        // A Deny permission rule yields PermissionDenied (no handler call).
        let mut registry = ToolRegistry::default();
        registry
            .register(echo_tool())
            .unwrap_or_else(|error| panic!("register: {error}"));
        let mut deny = PermissionRules::default();
        deny.push(PermissionRule::new(
            PermissionEffect::Deny,
            "tdw.echo",
            "tdw.echo",
        ));
        let orchestrator = ToolOrchestrator::new(registry, deny);
        assert_eq!(
            orchestrator.run(
                ToolCallId::new("c1").expect("tool call id"),
                "tdw.echo",
                json!({})
            ),
            Err(ToolError::PermissionDenied("tdw.echo".to_string()))
        );

        // Remaining validate_tool_definition branches.
        let base = || ToolDefinition {
            name: "tdw.echo".to_string(),
            description: "ok".to_string(),
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            permission_pattern: "tdw.echo".to_string(),
        };
        assert_eq!(
            validate_tool_definition(&ToolDefinition {
                description: "  ".to_string(),
                ..base()
            }),
            Err(ToolError::InvalidDefinition("description"))
        );
        // "tdw*" is not a valid pattern (a wildcard must follow a dot, e.g. "tdw.*").
        assert_eq!(
            validate_tool_definition(&ToolDefinition {
                permission_pattern: "tdw*".to_string(),
                ..base()
            }),
            Err(ToolError::InvalidDefinition("permission_pattern"))
        );
    }
}
