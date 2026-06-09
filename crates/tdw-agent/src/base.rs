//! The canonical `FinX` entity model — pure Rust, MCP-agnostic.
//!
//! These types are *ours*: identity/display ([`BaseMetadata`], [`Icon`]), classification
//! ([`Origin`], [`Adaptivity`]), value sourcing ([`Reference`]), and the shared
//! [`EntityMeta`] envelope. MCP compatibility is provided separately by the [`crate::mcp`]
//! adapter, which projects this model onto the MCP wire shape — so MCP is an edge concern,
//! not a driver of the core. `serde`/`schemars` here are only derive macros for our own
//! storage and schema export.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use validator::Validate;

use crate::kind::EntityKind;

/// A display icon descriptor (`src` + optional MIME type and size hints).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Icon {
    /// Source URI of the icon (`data:` or `https:`).
    pub src: String,
    /// Optional MIME type (e.g. `image/png`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Optional space-separated size hints (e.g. `"48x48 96x96"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sizes: Option<String>,
}

/// Identity and display metadata shared by every entity.
///
/// `name` is the stable programmatic identifier (used in API calls); `title` is the
/// mutable human-facing label. They must not be conflated. This shape is intentionally
/// MCP-`BaseMetadata`-compatible so the [`crate::mcp`] adapter can reuse it without a
/// second type, but it is defined here as part of our canonical model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct BaseMetadata {
    /// Unique programmatic identifier; also the display fallback.
    #[validate(length(min = 1))]
    pub name: String,
    /// Optional human-readable display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional human-readable description (a model-facing hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional display icons.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<Icon>,
}

impl BaseMetadata {
    /// Display name: `title` if present, otherwise `name`.
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

/// Origin tier — how built-in or specialized an entity kind is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Tier {
    /// Platform primitive.
    System,
    /// Finance/trading-domain specific.
    Domain,
    /// User-authored at runtime.
    Custom,
}

/// Origin source — whether the kind is defined inside the platform or bridges outside.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Source {
    /// Defined inside the platform/user.
    Internal,
    /// Wraps or mirrors a third-party standard or service.
    External,
}

/// Orthogonal entity classification.
///
/// This is NOT lineage: where a particular piece of *data* came from is recorded by a
/// separate `provenance` field. `Origin` classifies the kind itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Origin {
    /// How built-in/specialized the kind is.
    pub tier: Tier,
    /// Whether the kind bridges outside the platform.
    pub source: Source,
}

/// A tool's effect on the world — the domain effect classification.
///
/// MCP-agnostic: the [`crate::mcp`] adapter maps this to MCP `ToolAnnotations`
/// (`readOnlyHint`/`destructiveHint`) at the wire boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ToolEffect {
    /// Does not modify state; safe to run concurrently.
    ReadOnly,
    /// Modifies state, but safely/idempotently. The conservative default for a tool that
    /// does not declare an effect: not parallelized, not flagged destructive.
    #[default]
    WriteSafe,
    /// Performs destructive updates; must be serialized and confirmed.
    Destructive,
}

/// Ordinal degree to which an entity changes itself over time.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum Adaptivity {
    /// Static.
    None,
    /// Parameterized but not self-changing.
    Configured,
    /// Learns/updates from feedback or data.
    Learning,
    /// Rewrites its own structure (e.g. memory consolidation).
    SelfModifying,
}

/// The feedback gate's minimum adaptivity for eval-driven mutations.
///
/// An entity must be at least [`Adaptivity::Learning`] to receive eval-driven mutations.
/// A `None`/`Configured` entity is static or merely parameterized, so applying learned
/// feedback to it would be a contract violation.
pub const FEEDBACK_MIN_ADAPTIVITY: Adaptivity = Adaptivity::Learning;

/// Error returned by [`ensure_adaptive_for_feedback`] when an entity is not adaptive enough
/// to receive eval feedback.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AdaptivityError {
    /// The entity's adaptivity is below [`Adaptivity::Learning`], so it may not be mutated
    /// by feedback.
    #[error(
        "entity '{name}' is not learning-adaptive (adaptivity: {adaptivity:?}); cannot receive eval feedback"
    )]
    NotLearning {
        /// The offending entity's programmatic name.
        name: String,
        /// The entity's declared adaptivity.
        adaptivity: Adaptivity,
    },
}

/// The adaptivity gate for eval feedback: succeeds iff <code>meta.adaptivity >= [Adaptivity::Learning]</code>.
///
/// This guards every feedback mutation so that only `Learning`/`SelfModifying` entities accrue
/// learned state; a `None`/`Configured` entity is rejected with [`AdaptivityError::NotLearning`].
///
/// # Errors
///
/// Returns [`AdaptivityError::NotLearning`] when `meta.adaptivity < Adaptivity::Learning`.
pub fn ensure_adaptive_for_feedback(meta: &EntityMeta) -> Result<(), AdaptivityError> {
    if meta.adaptivity >= FEEDBACK_MIN_ADAPTIVITY {
        Ok(())
    } else {
        Err(AdaptivityError::NotLearning {
            name: meta.base.name.clone(),
            adaptivity: meta.adaptivity,
        })
    }
}

/// Human-memory-inspired retention tier for the `memory` kind.
///
/// Ordinal: shorter-lived tiers sort before longer-lived ones. Grounds on the `.remember/`
/// layout (working → short → mid → long → core/forever).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
pub enum Retention {
    /// In-context working buffer; ephemeral.
    Working,
    /// Session/day scope. The default for an undeclared memory.
    #[default]
    ShortTerm,
    /// Days–weeks; consolidating.
    MidTerm,
    /// Persistent, consolidated.
    LongTerm,
    /// Permanent identity memory ("forever").
    Core,
}

impl Retention {
    /// Approximate time-to-live in days; `None` means it never expires (`Core`).
    #[must_use]
    pub const fn ttl_days(self) -> Option<u32> {
        match self {
            Self::Working => Some(0),
            Self::ShortTerm => Some(1),
            Self::MidTerm => Some(7),
            Self::LongTerm => Some(90),
            Self::Core => None,
        }
    }

    /// The next longer-lived tier a surviving memory consolidates into. `Core` is the
    /// fixpoint (consolidating a `Core` memory leaves it `Core`).
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Working => Self::ShortTerm,
            Self::ShortTerm => Self::MidTerm,
            Self::MidTerm => Self::LongTerm,
            Self::LongTerm | Self::Core => Self::Core,
        }
    }
}

/// A value that is either a literal or a deferred reference resolved at runtime.
///
/// The single "value sourcing" primitive: any schema field can hold a `Reference` so it
/// can later resolve from variables, files, GitHub, HTTP, connectors, or secrets without
/// changing the field's type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum Reference {
    /// An inline literal value.
    Literal(Value),
    /// A named variable (e.g. an `environmentvariable`).
    Variable(String),
    /// A local file path.
    File(String),
    /// A GitHub/git reference (`repo@ref:path`).
    Git(String),
    /// An HTTP(S) URL.
    Http(String),
    /// A `connector` integration handle.
    Connector(String),
    /// A `secretstore` handle.
    Secret(String),
}

impl Reference {
    /// The [`Source`] implied by this reference's resolution channel.
    ///
    /// Git / HTTP / connector references bridge outside the platform ([`Source::External`]);
    /// literals, variables and files are [`Source::Internal`].
    #[must_use]
    pub const fn source(&self) -> Source {
        match self {
            Self::Git(_) | Self::Http(_) | Self::Connector(_) => Source::External,
            Self::Literal(_)
            | Self::Variable(_)
            | Self::File(_)
            // Secrets resolve through a platform-mediated `secretstore`, so the reference
            // is `Internal` even if the backing vault is an external provider.
            | Self::Secret(_) => Source::Internal,
        }
    }
}

/// How a `tool` is executed — a pluggable execution binding.
///
/// `Ref` resolves a registry implementation entity (the plugin path: a plugin ships a
/// `function`/`connector`/… entity and a `tool` that references it); the inline variants
/// carry the strategy directly. `Unbound` (the default) means the tool is listed but not
/// yet runnable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum ToolImplementation {
    /// Not yet bound: listed (annotated not-executable), `-32601` on call.
    #[default]
    Unbound,
    /// In-process Rust handler registered by name in a tool registry.
    Builtin {
        /// The registered handler name.
        handler: String,
    },
    /// Spawn an OS process. `background: true` returns a job id (poll/kill); otherwise the
    /// call captures output and returns it.
    Command {
        /// The program to run.
        command: String,
        /// Arguments (no shell expansion).
        args: Vec<String>,
        /// Run detached as a background job rather than capture-and-return.
        background: bool,
    },
    /// An interactive terminal session (`ConPTY` via `portable-pty`).
    Pty {
        /// The shell/program to launch.
        command: String,
        /// Arguments.
        args: Vec<String>,
    },
    /// HTTP POST to an endpoint (resolved via R1).
    Http {
        /// The endpoint reference.
        url: Reference,
    },
    /// Proxy to a tool on a sub-MCP server entity.
    Mcp {
        /// The `mcpserver` entity name.
        server: String,
        /// The tool name on that server.
        tool_name: String,
    },
    /// A sandboxed WASM module (resolved via R1).
    Wasm {
        /// The module reference.
        module: Reference,
    },
    /// Resolve a pluggable implementation entity from the registry by kind + name.
    Ref {
        /// The implementation entity's kind.
        kind: EntityKind,
        /// The implementation entity's name.
        name: String,
    },
}

/// The shared metadata envelope embedded by every entity kind (the CRD `metadata`).
///
/// Carries [`BaseMetadata`] (flattened, so `name`/`title`/… sit at the top level) plus the
/// `FinX` classification and a forward-compatible `_meta` bag for unknown fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EntityMeta {
    /// Identity & display metadata.
    #[serde(flatten)]
    #[validate(nested)]
    pub base: BaseMetadata,
    /// Internal stable identifier (ULID-shaped).
    #[validate(length(min = 1))]
    pub id: String,
    /// Semantic version of this definition.
    #[validate(length(min = 1))]
    pub version: String,
    /// Orthogonal classification.
    pub origin: Origin,
    /// Degree of self-change.
    pub adaptivity: Adaptivity,
    /// Whether this entity is an autonomous actor (true only for agents).
    pub autonomous: bool,
    /// Free-form tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Forward-compatibility bag for unknown/future metadata fields.
    #[serde(rename = "_meta", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, Value>,
}

impl EntityMeta {
    /// Construct an envelope with the common fields; `title`/`description`/`tags`/`_meta`
    /// default empty. Use the `with_*` builders to fill the rest.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        id: impl Into<String>,
        version: impl Into<String>,
        origin: Origin,
        adaptivity: Adaptivity,
        autonomous: bool,
    ) -> Self {
        Self {
            base: BaseMetadata {
                name: name.into(),
                title: None,
                description: None,
                icons: Vec::new(),
            },
            id: id.into(),
            version: version.into(),
            origin,
            adaptivity,
            autonomous,
            tags: Vec::new(),
            meta: BTreeMap::new(),
        }
    }

    /// Set the human display title.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.base.title = Some(title.into());
        self
    }

    /// Set the human description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.base.description = Some(description.into());
        self
    }

    /// Set the tags.
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Validate the embedded [`BaseMetadata`] in addition to this envelope's own fields.
    ///
    /// # Errors
    ///
    /// Returns a [`validator::ValidationErrors`] if either the base metadata or the
    /// envelope fields fail validation.
    pub fn validate_all(&self) -> Result<(), validator::ValidationErrors> {
        self.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn base_display_name_prefers_title() {
        let with_title = BaseMetadata {
            name: "the_id".to_string(),
            title: Some("The Title".to_string()),
            description: None,
            icons: Vec::new(),
        };
        assert_eq!(with_title.display_name(), "The Title");
        let no_title = BaseMetadata {
            title: None,
            ..with_title
        };
        assert_eq!(no_title.display_name(), "the_id");
    }

    #[test]
    fn reference_source_maps_external_channels() {
        assert_eq!(
            Reference::Git("a@b:c".to_string()).source(),
            Source::External
        );
        assert_eq!(
            Reference::Http("https://x".to_string()).source(),
            Source::External
        );
        assert_eq!(
            Reference::Connector("warehouse".to_string()).source(),
            Source::External
        );
        assert_eq!(
            Reference::File("./x".to_string()).source(),
            Source::Internal
        );
        assert_eq!(
            Reference::Variable("X".to_string()).source(),
            Source::Internal
        );
        assert_eq!(Reference::Literal(json!(1)).source(), Source::Internal);
        // Secret is the counter-intuitive case: Internal even though the backing
        // vault may be an external provider. Pin it so a reclassification to
        // External (which would change Origin source semantics) fails CI.
        assert_eq!(
            Reference::Secret("vault/key".to_string()).source(),
            Source::Internal
        );
    }

    #[test]
    fn retention_is_ordinal_with_ttls() {
        assert!(Retention::Working < Retention::ShortTerm);
        assert!(Retention::MidTerm < Retention::LongTerm);
        assert!(Retention::LongTerm < Retention::Core);
        assert_eq!(Retention::ShortTerm.ttl_days(), Some(1));
        assert_eq!(Retention::LongTerm.ttl_days(), Some(90));
        assert_eq!(Retention::Core.ttl_days(), None);
        assert_eq!(Retention::default(), Retention::ShortTerm);
    }

    #[test]
    fn adaptivity_is_ordinal() {
        assert!(Adaptivity::None < Adaptivity::Configured);
        assert!(Adaptivity::Configured < Adaptivity::Learning);
        assert!(Adaptivity::Learning < Adaptivity::SelfModifying);
    }

    #[test]
    fn entity_meta_builders_and_round_trip() {
        let meta = EntityMeta::new(
            "market_researcher",
            "01J0AGENT",
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::Learning,
            true,
        )
        .with_title("Market Researcher")
        .with_description("Drafts research notes.")
        .with_tags(vec!["research".to_string()]);
        assert!(meta.validate_all().is_ok());

        let encoded = serde_json::to_value(&meta).expect("EntityMeta should serialize");
        // BaseMetadata is flattened: name sits at the top level, not under `base`.
        assert_eq!(
            encoded.get("name").and_then(Value::as_str),
            Some("market_researcher")
        );
        assert!(encoded.get("base").is_none());
        assert_eq!(
            encoded.get("title").and_then(Value::as_str),
            Some("Market Researcher")
        );

        let decoded: EntityMeta =
            serde_json::from_value(encoded).expect("EntityMeta should deserialize");
        assert_eq!(decoded, meta);
    }

    #[test]
    fn entity_meta_rejects_empty_name_via_nested_validation() {
        let meta = EntityMeta::new(
            "",
            "id1",
            "0.1.0",
            Origin {
                tier: Tier::System,
                source: Source::Internal,
            },
            Adaptivity::None,
            false,
        );
        assert!(meta.validate_all().is_err());
    }

    #[test]
    fn entity_meta_rejects_empty_id_and_version() {
        let origin = Origin {
            tier: Tier::System,
            source: Source::Internal,
        };
        // Empty `id` — a top-level validator, a different path from the nested
        // `name` one already covered.
        let empty_id = EntityMeta::new("valid_name", "", "0.1.0", origin, Adaptivity::None, false);
        assert!(
            empty_id.validate_all().is_err(),
            "empty id must be rejected"
        );
        // Empty `version`.
        let empty_version =
            EntityMeta::new("valid_name", "id1", "", origin, Adaptivity::None, false);
        assert!(
            empty_version.validate_all().is_err(),
            "empty version must be rejected"
        );
    }
}
