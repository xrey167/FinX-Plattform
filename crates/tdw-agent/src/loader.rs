//! JSON5 registry loader.
//!
//! Definition files are authored in JSON5 — JSON syntax plus comments, unquoted keys, and
//! trailing commas — which is pleasant for hand-editing the registry. This is the
//! *authoring/storage* format only; the MCP wire stays strict JSON via the [`crate::mcp`]
//! adapter. Because the canonical types derive `serde`, the same types load from JSON5 here
//! and serialize to strict JSON at the wire boundary.

use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

use crate::kind::EntityKind;
use crate::resource::Resource;

/// Error loading a registry definition from JSON5.
#[derive(Debug, Error)]
pub enum LoadError {
    /// The text was not valid JSON5 for the target type.
    #[error("json5 parse error: {0}")]
    Parse(String),
    /// A filesystem error while reading a definition file or directory.
    #[error("io error reading {path}: {source}")]
    Io {
        /// The path that failed.
        path: String,
        /// The underlying IO error.
        #[source]
        source: std::io::Error,
    },
    /// Two definitions share the same `(kind, name)` key.
    #[error("duplicate definition: {kind:?} / {name}")]
    Duplicate {
        /// The conflicting kind.
        kind: EntityKind,
        /// The conflicting name.
        name: String,
    },
    /// A resource violates a taxonomy invariant (e.g. autonomy on a non-agent kind).
    #[error("invariant violation for {kind:?}: {detail}")]
    Invariant {
        /// The offending kind.
        kind: EntityKind,
        /// What invariant was violated.
        detail: String,
    },
    /// A `plugin` declares a member that is not present in the registry.
    #[error("plugin {plugin} references missing member: {member}")]
    MissingMember {
        /// The plugin declaring the member.
        plugin: String,
        /// The unresolved member name.
        member: String,
    },
}

/// Parse a CRD [`Resource`] envelope (`apiVersion`/`kind`/`metadata`/`spec`) from JSON5.
///
/// # Errors
///
/// Returns [`LoadError::Parse`] if the text is not valid JSON5 for a [`Resource`].
pub fn load_resource(text: &str) -> Result<Resource<Value>, LoadError> {
    json5::from_str(text).map_err(|error| LoadError::Parse(error.to_string()))
}

/// Parse a typed entity (e.g. [`crate::AgentSkill`]) directly from JSON5.
///
/// # Errors
///
/// Returns [`LoadError::Parse`] if the text is not valid JSON5 for `T`.
pub fn load_typed<T: DeserializeOwned>(text: &str) -> Result<T, LoadError> {
    json5::from_str(text).map_err(|error| LoadError::Parse(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentSkill, EntityKind};

    #[test]
    fn loads_resource_from_json5_with_comments() {
        // Comments, unquoted keys, and trailing commas — none of which strict JSON allows.
        let text = r#"
        {
            // a hand-authored tool resource
            apiVersion: "tdw.finx/v1",
            kind: "tool",
            metadata: {
                name: "search",
                id: "search",
                version: "0.1.0",
                origin: { tier: "Domain", source: "Internal" },
                adaptivity: "None",
                autonomous: false, /* not an actor */
            },
            spec: {
                inputSchema: { type: "object" },
            },
        }
        "#;

        let resource = load_resource(text).expect("json5 resource should parse");
        assert_eq!(resource.kind, EntityKind::Tool);
        assert_eq!(resource.api_version, "tdw.finx/v1");
        assert_eq!(resource.metadata.id, "search");
        assert_eq!(resource.metadata.base.name, "search");
    }

    #[test]
    fn loads_typed_entity_from_json5() {
        let text = r#"
        {
            meta: {
                name: "research.note",
                title: "Research Note",
                id: "research.note",
                version: "0.1.0",
                origin: { tier: "Domain", source: "Internal" },
                adaptivity: "Configured",
                autonomous: false,
                tags: ["research", "mcp"],
            },
            input_schema: { type: "object" },
            output_schema: { type: "object" },
        }
        "#;

        let skill: AgentSkill = load_typed(text).expect("json5 skill should parse");
        assert_eq!(skill.meta.id, "research.note");
        assert_eq!(skill.meta.base.title.as_deref(), Some("Research Note"));
        assert_eq!(skill.meta.tags, vec!["research", "mcp"]);
    }

    #[test]
    fn parse_error_is_reported() {
        let result = load_resource("{ this is : not json5 ");
        assert!(matches!(result, Err(LoadError::Parse(_))));
    }
}
