//! Cross-cutting facets attached to specific kinds.
//!
//! - [`DataFacets`] — on data/content kinds (track B): plane, materialization, a
//!   point-in-time `as_of`, and a validation gate.
//! - [`EvalFacets`] — on the `evaluation` kind (track A): the ML rigor for evaluating
//!   agent skills (baselines, fixed splits, slices, drift, failure attribution, ops).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::Validate;

/// Which plane owns a data/content entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Plane {
    /// Worker knowledge/memory/features built and used at runtime.
    Agent,
    /// Knowledge/features in the trading data warehouse.
    Platform,
    /// Shared by both.
    Shared,
}

/// How a data/content entity physically exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Materialization {
    /// Built up and stored inside the platform.
    Materialized,
    /// Queried live from a warehouse via a connector/datastore.
    Federated,
    /// Cached/mixed.
    Hybrid,
}

/// Validation-gate status for a data/content entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ValidationStatus {
    /// Not yet validated.
    Draft,
    /// Passed validation.
    Validated,
    /// Validated and approved for use.
    Ready,
}

/// Result of validating a data/content entity (the data-prep gate).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ValidationState {
    /// Gate status.
    pub status: ValidationStatus,
    /// Fraction of missing/NULL values, if measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[validate(range(min = 0.0, max = 1.0))]
    pub missing_fraction: Option<f64>,
    /// Whether the data conforms to its declared schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_conformant: Option<bool>,
    /// When the entity was last validated (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validated: Option<String>,
}

/// Facets carried by data/content kinds (track B).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct DataFacets {
    /// Which plane owns this entity.
    pub plane: Plane,
    /// How the data physically exists.
    pub materialization: Materialization,
    /// Point-in-time the data is valid as of (RFC 3339); leakage-safe key. `None` for
    /// non-temporal data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<String>,
    /// Validation-gate state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[validate(nested)]
    pub validation: Option<ValidationState>,
}

/// Operational metrics for an evaluation run (track A6).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct OpsMetrics {
    /// End-to-end latency in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    /// Cost in USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Input tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_in: Option<u64>,
    /// Output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_out: Option<u64>,
    /// Calibration score (0..=1), if measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[validate(range(min = 0.0, max = 1.0))]
    pub calibration: Option<f64>,
}

/// ML-rigor facets for the `evaluation` kind (track A: evaluating agent skills).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase")]
pub struct EvalFacets {
    /// A1 — references to baseline agents/models every candidate is compared against.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub baselines: Vec<String>,
    /// A2 — identifier of the fixed split shared by all compared agents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_split_id: Option<String>,
    /// A3 — dimensions to slice metrics by (e.g. `task_type`, `difficulty`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slice_dimensions: Vec<String>,
    /// A4 — whether the leaderboard tracks quality drift across versions/time.
    #[serde(default)]
    pub tracks_drift: bool,
    /// A5 — references to `gotcha` ids capturing observed failure modes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_mode_refs: Vec<String>,
    /// A6 — operational trade-off metrics.
    #[serde(default)]
    #[validate(nested)]
    pub ops: OpsMetrics,
    /// A7 — whether a chronological (leakage-safe) split is used for temporal eval data.
    #[serde(default)]
    pub chronological_split: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_status_is_ordinal() {
        assert!(ValidationStatus::Draft < ValidationStatus::Validated);
        assert!(ValidationStatus::Validated < ValidationStatus::Ready);
    }

    #[test]
    fn data_facets_round_trip_camel_case() {
        let facets = DataFacets {
            plane: Plane::Platform,
            materialization: Materialization::Federated,
            as_of: Some("2026-05-31T00:00:00Z".to_string()),
            validation: Some(ValidationState {
                status: ValidationStatus::Ready,
                missing_fraction: Some(0.0),
                schema_conformant: Some(true),
                last_validated: Some("2026-05-31T00:00:00Z".to_string()),
            }),
        };
        let encoded = serde_json::to_value(&facets).expect("facets should serialize");
        assert_eq!(
            encoded.get("plane").and_then(serde_json::Value::as_str),
            Some("platform")
        );
        assert!(encoded.get("asOf").is_some());
        assert!(
            encoded
                .get("validation")
                .and_then(|v| v.get("schemaConformant"))
                .is_some()
        );
        let decoded: DataFacets =
            serde_json::from_value(encoded).expect("facets should deserialize");
        assert_eq!(decoded, facets);
    }

    #[test]
    fn eval_facets_default_is_empty_but_present() {
        let facets = EvalFacets::default();
        assert!(facets.baselines.is_empty());
        assert!(!facets.tracks_drift);
        let encoded = serde_json::to_value(&facets).expect("eval facets should serialize");
        // ops always present (not skipped); tracksDrift present as false.
        assert!(encoded.get("ops").is_some());
        assert_eq!(
            encoded.get("tracksDrift").and_then(serde_json::Value::as_bool),
            Some(false)
        );
    }
}
