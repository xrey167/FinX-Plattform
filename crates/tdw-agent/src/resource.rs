//! The CRD-style resource envelope: `apiVersion` + `kind` + `metadata` + `spec` + `status`.
//!
//! `Resource<S>` wraps any kind-specific `spec` with the shared [`EntityMeta`] envelope and
//! an [`EntityKind`] discriminator, so every entity has a uniform outer shape (the
//! Kubernetes-CRD pattern). Existing concrete schemas can be migrated to `spec` types
//! incrementally without changing this envelope.

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::base::EntityMeta;
use crate::kind::{EntityKind, Group};

/// Default API version for FinX agent-platform resources.
pub const TDW_API_VERSION: &str = "tdw.finx/v1";

/// API group for FinX resource definitions.
pub const TDW_API_GROUP: &str = "tdw.finx";

/// The metadata field name lifted into / merged out of the [`Resource`] envelope.
///
/// Every entity struct MUST name its metadata field `meta`; [`RegistryEntity::to_resource`]
/// and [`entity_from_resource`] rely on this to move the [`EntityMeta`] between the embedded
/// spec form and the CRD envelope.
pub const META_FIELD: &str = "meta";

/// A meta-schema entry describing one entity kind — the registry's CRD analog.
///
/// Makes the registry self-describing: each kind is paired with its manifest group, the
/// JSON Schema of its `spec` (when a concrete Rust type exists), and which cross-cutting
/// facets apply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceDefinition {
    /// API group (e.g. `tdw.finx`).
    pub group: String,
    /// The kind this definition describes.
    pub kind: EntityKind,
    /// Manifest group the kind belongs to.
    pub manifest_group: Group,
    /// JSON Schema (2020-12) for the kind's `spec`, when a concrete type exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_schema: Option<Value>,
    /// Whether instances carry the data facets (plane/materialization/as_of/validation).
    pub has_data_facets: bool,
    /// Whether the kind is autonomy-capable (agents only).
    pub autonomy_capable: bool,
}

/// A CRD-style resource: outer envelope shared by every entity kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resource<S> {
    /// API version (e.g. `tdw.finx/v1`).
    pub api_version: String,
    /// The entity kind discriminator.
    pub kind: EntityKind,
    /// Shared metadata envelope (MCP base + FinX classification).
    pub metadata: EntityMeta,
    /// Kind-specific payload.
    pub spec: S,
    /// Optional runtime state (executable kinds only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Value>,
}

impl<S> Resource<S> {
    /// Build a resource at the default [`TDW_API_VERSION`] with no status.
    pub fn new(kind: EntityKind, metadata: EntityMeta, spec: S) -> Self {
        Self {
            api_version: TDW_API_VERSION.to_string(),
            kind,
            metadata,
            spec,
            status: None,
        }
    }
}

/// An entity that embeds an [`EntityMeta`] and can be projected onto the CRD
/// [`Resource`] envelope at the registry boundary.
///
/// Implementors keep `meta` embedded for in-memory use; [`RegistryEntity::to_resource`]
/// lifts that metadata into the envelope so the `spec` is pure payload (no duplicated
/// metadata) — the uniform shape used for storage and MCP round-tripping.
pub trait RegistryEntity: Serialize {
    /// The kind discriminator for this entity type.
    const KIND: EntityKind;

    /// Borrow this entity's metadata envelope.
    fn metadata(&self) -> &EntityMeta;

    /// Project into a [`Resource`]: metadata lifted out, `spec` = the remaining payload.
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if the entity fails to serialize.
    fn to_resource(&self) -> Result<Resource<Value>, serde_json::Error> {
        let mut spec = serde_json::to_value(self)?;
        if let Some(object) = spec.as_object_mut() {
            object.remove(META_FIELD);
        }
        Ok(Resource::new(Self::KIND, self.metadata().clone(), spec))
    }
}

/// Rebuild a typed entity from a [`Resource`] by merging the envelope `metadata` back into
/// the `spec` — the inverse of [`RegistryEntity::to_resource`]. Lets a generic
/// `Resource<Value>` loaded from the registry be re-typed (e.g. into a `Tool`/`Prompt`).
///
/// # Errors
///
/// Returns a [`serde_json::Error`] if the merged value does not deserialize into `T`.
pub fn entity_from_resource<T: serde::de::DeserializeOwned>(
    resource: &Resource<Value>,
) -> Result<T, serde_json::Error> {
    let mut spec = resource.spec.clone();
    let Some(object) = spec.as_object_mut() else {
        return Err(serde_json::Error::custom(
            "resource spec must be a JSON object",
        ));
    };
    object.insert(
        META_FIELD.to_string(),
        serde_json::to_value(&resource.metadata)?,
    );
    serde_json::from_value(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::{Adaptivity, BaseMetadata, Origin, Source, Tier};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn sample_meta() -> EntityMeta {
        EntityMeta {
            base: BaseMetadata {
                name: "research_note".to_string(),
                title: Some("Research Note".to_string()),
                description: None,
                icons: Vec::new(),
            },
            id: "01J0SKILL".to_string(),
            version: "0.1.0".to_string(),
            origin: Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            adaptivity: Adaptivity::Configured,
            autonomous: false,
            tags: Vec::new(),
            meta: BTreeMap::new(),
        }
    }

    #[test]
    fn resource_wraps_spec_and_round_trips() {
        let resource = Resource::new(
            EntityKind::Skill,
            sample_meta(),
            json!({"inputSchema": {"type": "object"}}),
        );
        assert_eq!(resource.api_version, "tdw.finx/v1");

        let encoded = serde_json::to_value(&resource).expect("resource should serialize");
        // CRD-shaped: apiVersion/kind/metadata/spec at the top level.
        assert_eq!(
            encoded.get("apiVersion").and_then(Value::as_str),
            Some("tdw.finx/v1")
        );
        assert_eq!(encoded.get("kind").and_then(Value::as_str), Some("skill"));
        assert!(encoded.get("metadata").is_some());
        assert!(encoded.get("spec").is_some());
        // status omitted when None.
        assert!(encoded.get("status").is_none());

        let decoded: Resource<Value> =
            serde_json::from_value(encoded).expect("resource should deserialize");
        assert_eq!(decoded, resource);
    }

    #[test]
    fn entity_from_resource_errors_on_non_object_spec() {
        // A spec that is not a JSON object cannot carry merged metadata: error, never drop it.
        let resource = Resource::new(EntityKind::Skill, sample_meta(), json!("not an object"));
        let result: Result<crate::AgentSkill, _> = entity_from_resource(&resource);
        assert!(result.is_err());
    }
}
