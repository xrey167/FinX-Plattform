//! MCP wire adapter (the edge).
//!
//! This module holds the MCP-spec-specific representations — the `Tool` wire shape, its
//! annotations, the icon MIME-support tiers, and the targeted protocol revision. The
//! canonical model lives in [`crate::base`]; this adapter projects it onto / from the MCP
//! wire format (camelCase, `inputSchema`/`outputSchema`, …) so MCP changes stay contained
//! here rather than rippling into the domain. Serialization naming that differs from our
//! own idiomatic naming belongs in this layer.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::base::BaseMetadata;
use crate::kind::EntityKind;
use crate::resource::{Resource, entity_from_resource};

/// The MCP protocol revision this adapter targets.
pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

/// Client support tier for an icon MIME type, per MCP `2025-11-25`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum IconMimeSupport {
    /// Clients MUST support (`image/png`, `image/jpeg`).
    Must,
    /// Clients SHOULD support (`image/svg+xml`, `image/webp`).
    Should,
    /// Outside the baseline support set.
    Optional,
}

/// Classify an icon MIME type into its MCP client-support tier.
#[must_use]
pub fn icon_mime_support(mime_type: &str) -> IconMimeSupport {
    match mime_type {
        "image/png" | "image/jpeg" => IconMimeSupport::Must,
        "image/svg+xml" | "image/webp" => IconMimeSupport::Should,
        _ => IconMimeSupport::Optional,
    }
}

/// MCP `ToolAnnotations` — behavioral hints that also drive execution scheduling.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    /// Human display title (takes precedence over `name` for tools).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The tool does not modify its environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    /// The tool may perform destructive updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    /// Repeated calls with the same arguments have no additional effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    /// The tool interacts with an open/external world.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
}

/// Whether a tool call may run concurrently with others or must be serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ParallelSafety {
    /// Safe to fan out concurrently.
    Parallel,
    /// Must run one at a time.
    Sequential,
}

/// An MCP `Tool` (revision `2025-11-25`): base metadata + I/O schemas + annotations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    /// MCP `BaseMetadata` (flattened to the top level), reused from the canonical model.
    #[serde(flatten)]
    pub base: BaseMetadata,
    /// JSON Schema (2020-12) for tool inputs; the root must be an object.
    pub input_schema: Value,
    /// Optional JSON Schema (2020-12) for `structuredContent` in `CallToolResult`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Optional behavioral annotations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
    /// Forward-compatibility bag for unknown/future MCP `_meta` fields.
    #[serde(rename = "_meta", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, Value>,
}

impl McpTool {
    /// Display name precedence for tools: `title` → `annotations.title` → `name`.
    #[must_use]
    pub fn display_name(&self) -> &str {
        if let Some(title) = self.base.title.as_deref() {
            return title;
        }
        if let Some(title) = self
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.title.as_deref())
        {
            return title;
        }
        &self.base.name
    }

    /// Execution scheduling derived from MCP annotations. Only `readOnlyHint` and
    /// `destructiveHint` drive the parallel/sequential decision today; the other three
    /// hints (`idempotentHint`/`openWorldHint`/`title`) do not affect scheduling.
    ///
    /// `destructiveHint` → [`ParallelSafety::Sequential`]; `readOnlyHint` →
    /// [`ParallelSafety::Parallel`]; otherwise the safe default of
    /// [`ParallelSafety::Sequential`].
    #[must_use]
    pub fn parallel_safety(&self) -> ParallelSafety {
        match self.annotations.as_ref() {
            Some(annotations) if annotations.destructive_hint == Some(true) => {
                ParallelSafety::Sequential
            }
            Some(annotations) if annotations.read_only_hint == Some(true) => {
                ParallelSafety::Parallel
            }
            _ => ParallelSafety::Sequential,
        }
    }

    /// Whether repeated calls are safe, which gates retry/backoff under an `errorpolicy`.
    #[must_use]
    pub fn is_idempotent(&self) -> bool {
        self.annotations
            .as_ref()
            .and_then(|annotations| annotations.idempotent_hint)
            == Some(true)
    }
}

impl From<&crate::Tool> for McpTool {
    /// Project a canonical [`crate::Tool`] onto the MCP wire shape, mapping the domain
    /// [`crate::ToolEffect`] to MCP annotations (`readOnlyHint`/`destructiveHint`).
    ///
    /// The effect is encoded explicitly so each variant is distinguishable on the wire:
    /// - `ReadOnly` → `readOnlyHint: Some(true)`, no destructive hint.
    /// - `WriteSafe` → `readOnlyHint: Some(false)`, no destructive hint (writes, but not
    ///   destructively).
    /// - `Destructive` → `readOnlyHint: Some(false)`, `destructiveHint: Some(true)`.
    fn from(tool: &crate::Tool) -> Self {
        let (read_only_hint, destructive_hint) = match tool.effect {
            crate::ToolEffect::ReadOnly => (Some(true), None),
            crate::ToolEffect::WriteSafe => (Some(false), None),
            crate::ToolEffect::Destructive => (Some(false), Some(true)),
        };
        let annotations = ToolAnnotations {
            title: None,
            read_only_hint,
            destructive_hint,
            idempotent_hint: tool.idempotent.then_some(true),
            open_world_hint: tool.open_world.then_some(true),
        };
        Self {
            base: tool.meta.base.clone(),
            input_schema: tool.input_schema.clone(),
            output_schema: tool.output_schema.clone(),
            annotations: Some(annotations),
            meta: BTreeMap::new(),
        }
    }
}

/// MCP `PromptArgument` — name + optional description + required flag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptArgument {
    /// Argument name.
    pub name: String,
    /// Optional human display title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional human description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the argument is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// An MCP `Prompt` (revision `2025-11-25`): base metadata + declared arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct McpPrompt {
    /// MCP `BaseMetadata` (flattened), reused from the canonical model.
    #[serde(flatten)]
    pub base: BaseMetadata,
    /// Declared arguments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<McpPromptArgument>,
    /// Forward-compatibility bag for unknown/future MCP `_meta` fields.
    #[serde(rename = "_meta", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, Value>,
}

impl From<&crate::Prompt> for McpPrompt {
    /// Project a canonical [`crate::Prompt`] onto the MCP wire shape. The argument
    /// `default` (an R1 reference) is domain-only and is not carried onto the wire.
    fn from(prompt: &crate::Prompt) -> Self {
        Self {
            base: prompt.meta.base.clone(),
            arguments: prompt
                .arguments
                .iter()
                .map(|argument| McpPromptArgument {
                    name: argument.name.clone(),
                    title: None,
                    description: argument.description.clone(),
                    required: Some(argument.required),
                })
                .collect(),
            meta: BTreeMap::new(),
        }
    }
}

/// An MCP-exposable entity projected from a registry [`Resource`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpEntity {
    /// An MCP tool.
    Tool(McpTool),
    /// An MCP prompt.
    Prompt(McpPrompt),
}

/// Project a registry [`Resource`] onto its MCP wire form, if the kind is MCP-exposable.
///
/// Only `tool` / `prompt` kinds are projected; other kinds return `None`. This completes
/// the round-trip JSON5 → `Resource` → canonical type → MCP wire.
///
/// # Errors
///
/// Returns a [`serde_json::Error`] if the resource does not re-type into the expected
/// canonical entity.
pub fn project_to_mcp(resource: &Resource<Value>) -> Result<Option<McpEntity>, serde_json::Error> {
    let entity = match resource.kind {
        EntityKind::Tool => Some(McpEntity::Tool(McpTool::from(&entity_from_resource::<
            crate::Tool,
        >(resource)?))),
        EntityKind::Prompt => Some(McpEntity::Prompt(McpPrompt::from(&entity_from_resource::<
            crate::Prompt,
        >(resource)?))),
        _ => None,
    };
    Ok(entity)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(name: &str, title: Option<&str>) -> BaseMetadata {
        BaseMetadata {
            name: name.to_string(),
            title: title.map(ToOwned::to_owned),
            description: None,
            icons: Vec::new(),
        }
    }

    #[test]
    fn icon_mime_tiers() {
        assert_eq!(icon_mime_support("image/png"), IconMimeSupport::Must);
        assert_eq!(icon_mime_support("image/jpeg"), IconMimeSupport::Must);
        // `image/jpg` is off-spec: it is not a MUST tier, only Optional.
        assert_eq!(icon_mime_support("image/jpg"), IconMimeSupport::Optional);
        assert_eq!(icon_mime_support("image/svg+xml"), IconMimeSupport::Should);
        assert_eq!(icon_mime_support("image/webp"), IconMimeSupport::Should);
        assert_eq!(icon_mime_support("image/gif"), IconMimeSupport::Optional);
    }

    #[test]
    fn write_safe_effect_is_explicit_on_the_wire() {
        let tool = crate::Tool {
            meta: crate::base::EntityMeta::new(
                "write_tool",
                "write_tool",
                "0.1.0",
                crate::base::Origin {
                    tier: crate::base::Tier::Domain,
                    source: crate::base::Source::Internal,
                },
                crate::base::Adaptivity::None,
                false,
            ),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            effect: crate::base::ToolEffect::WriteSafe,
            idempotent: false,
            open_world: false,
            implementation: crate::base::ToolImplementation::Unbound,
        };
        let mcp = McpTool::from(&tool);
        let annotations = mcp.annotations.as_ref().expect("annotations present");
        // WriteSafe is distinguishable from unknown: readOnly explicitly false, no destructive.
        assert_eq!(annotations.read_only_hint, Some(false));
        assert_eq!(annotations.destructive_hint, None);
    }

    #[test]
    fn tool_display_name_precedence() {
        let mut tool = McpTool {
            base: base("search_tool", None),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            annotations: None,
            meta: BTreeMap::new(),
        };
        assert_eq!(tool.display_name(), "search_tool");
        tool.annotations = Some(ToolAnnotations {
            title: Some("Annotated Title".to_string()),
            ..ToolAnnotations::default()
        });
        assert_eq!(tool.display_name(), "Annotated Title");
        tool.base.title = Some("Base Title".to_string());
        assert_eq!(tool.display_name(), "Base Title");
    }

    #[test]
    fn parallel_safety_follows_annotations() {
        let make = |annotations: Option<ToolAnnotations>| McpTool {
            base: base("t", None),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            annotations,
            meta: BTreeMap::new(),
        };
        assert_eq!(make(None).parallel_safety(), ParallelSafety::Sequential);
        assert_eq!(
            make(Some(ToolAnnotations {
                read_only_hint: Some(true),
                ..ToolAnnotations::default()
            }))
            .parallel_safety(),
            ParallelSafety::Parallel
        );
        assert_eq!(
            make(Some(ToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(true),
                ..ToolAnnotations::default()
            }))
            .parallel_safety(),
            ParallelSafety::Sequential
        );
    }

    #[test]
    fn mcp_tool_uses_camel_case_keys() {
        let tool = McpTool {
            base: base("search", Some("Search")),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: Some(serde_json::json!({"type": "object"})),
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(true),
                ..ToolAnnotations::default()
            }),
            meta: BTreeMap::new(),
        };
        let encoded = serde_json::to_value(&tool).expect("tool should serialize");
        assert!(encoded.get("inputSchema").is_some());
        assert!(encoded.get("outputSchema").is_some());
        assert_eq!(
            encoded
                .get("annotations")
                .and_then(|annotations| annotations.get("readOnlyHint"))
                .and_then(Value::as_bool),
            Some(true)
        );
        let decoded: McpTool = serde_json::from_value(encoded).expect("tool should deserialize");
        assert_eq!(decoded, tool);
    }

    fn sample_tool(effect: crate::base::ToolEffect, idempotent: bool) -> crate::Tool {
        crate::Tool {
            meta: crate::base::EntityMeta::new(
                "t",
                "t",
                "0.1.0",
                crate::base::Origin {
                    tier: crate::base::Tier::Domain,
                    source: crate::base::Source::Internal,
                },
                crate::base::Adaptivity::None,
                false,
            ),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            effect,
            idempotent,
            open_world: false,
            implementation: crate::base::ToolImplementation::Unbound,
        }
    }

    #[test]
    fn from_tool_maps_read_only_and_destructive_effects() {
        // Only the WriteSafe arm was covered; pin the security-relevant arms.
        let read_only = McpTool::from(&sample_tool(crate::base::ToolEffect::ReadOnly, false));
        let annotations = read_only.annotations.as_ref().expect("annotations present");
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.destructive_hint, None);

        let destructive = McpTool::from(&sample_tool(crate::base::ToolEffect::Destructive, false));
        let annotations = destructive
            .annotations
            .as_ref()
            .expect("annotations present");
        assert_eq!(annotations.read_only_hint, Some(false));
        assert_eq!(annotations.destructive_hint, Some(true));
    }

    #[test]
    fn is_idempotent_follows_idempotent_hint() {
        let with_hint = |hint: bool| McpTool {
            base: base("t", None),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                idempotent_hint: Some(hint),
                ..ToolAnnotations::default()
            }),
            meta: BTreeMap::new(),
        };
        assert!(with_hint(true).is_idempotent());
        assert!(!with_hint(false).is_idempotent());

        // No annotations, and annotations present but no idempotent hint, are
        // both non-idempotent (no retry/backoff).
        let no_annotations = McpTool {
            base: base("t", None),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: None,
            annotations: None,
            meta: BTreeMap::new(),
        };
        assert!(!no_annotations.is_idempotent());
        let annotations_no_hint = McpTool {
            annotations: Some(ToolAnnotations::default()),
            ..no_annotations
        };
        assert!(!annotations_no_hint.is_idempotent());
    }

    #[test]
    fn from_prompt_projects_arguments_and_drops_domain_only_fields() {
        let prompt = crate::Prompt {
            meta: crate::base::EntityMeta::new(
                "greet",
                "greet",
                "0.1.0",
                crate::base::Origin {
                    tier: crate::base::Tier::Domain,
                    source: crate::base::Source::Internal,
                },
                crate::base::Adaptivity::None,
                false,
            ),
            template: "Hello {{ name }}".to_string(),
            arguments: vec![crate::PromptArgument {
                name: "name".to_string(),
                description: Some("the name".to_string()),
                required: true,
                default: None,
            }],
        };
        let mcp = McpPrompt::from(&prompt);
        assert_eq!(mcp.arguments.len(), 1);
        let argument = &mcp.arguments[0];
        assert_eq!(argument.name, "name");
        assert_eq!(argument.description.as_deref(), Some("the name"));
        assert_eq!(argument.required, Some(true));
        // title is wire-only and not derived from the domain argument.
        assert_eq!(argument.title, None);
    }

    #[test]
    fn project_to_mcp_exposes_tool_and_prompt_only() {
        use crate::RegistryEntity;

        // tool kind → Some(Tool).
        let tool_resource = sample_tool(crate::base::ToolEffect::ReadOnly, true)
            .to_resource()
            .expect("tool projects to a resource");
        assert!(matches!(
            project_to_mcp(&tool_resource),
            Ok(Some(McpEntity::Tool(_)))
        ));

        // prompt kind → Some(Prompt).
        let prompt = crate::Prompt {
            meta: crate::base::EntityMeta::new(
                "p",
                "p",
                "0.1.0",
                crate::base::Origin {
                    tier: crate::base::Tier::Domain,
                    source: crate::base::Source::Internal,
                },
                crate::base::Adaptivity::None,
                false,
            ),
            template: "x".to_string(),
            arguments: Vec::new(),
        };
        let prompt_resource = prompt.to_resource().expect("prompt projects to a resource");
        assert!(matches!(
            project_to_mcp(&prompt_resource),
            Ok(Some(McpEntity::Prompt(_)))
        ));

        // A non-exposable kind (agent) → None (the `_` arm).
        let agent_resource = Resource::new(
            EntityKind::Agent,
            crate::base::EntityMeta::new(
                "a",
                "a",
                "0.1.0",
                crate::base::Origin {
                    tier: crate::base::Tier::Domain,
                    source: crate::base::Source::Internal,
                },
                crate::base::Adaptivity::None,
                false,
            ),
            serde_json::json!({}),
        );
        assert!(matches!(project_to_mcp(&agent_resource), Ok(None)));
    }
}
