#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tdw_tags::TagStore;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureSnapshot {
    pub entity_id: String,
    pub as_of: String,
    pub features: BTreeMap<String, f64>,
    pub tags: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureError {
    InvalidEntityId,
    InvalidAsOf,
    InvalidFeatureName,
    NonFiniteFeatureValue,
}

#[derive(Clone, Debug, Default)]
pub struct FeatureStore {
    snapshots: Vec<FeatureSnapshot>,
}

impl FeatureStore {
    pub fn materialize(
        &mut self,
        entity_id: &str,
        as_of: &str,
        features: BTreeMap<String, f64>,
        tags: &TagStore,
    ) -> FeatureSnapshot {
        let snapshot = FeatureSnapshot {
            entity_id: entity_id.to_string(),
            as_of: as_of.to_string(),
            features,
            tags: tags.active_tags(entity_id, as_of),
        };
        self.snapshots.push(snapshot.clone());
        snapshot
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn try_materialize(
        &mut self,
        entity_id: &str,
        as_of: &str,
        features: BTreeMap<String, f64>,
        tags: &TagStore,
    ) -> Result<FeatureSnapshot, FeatureError> {
        validate_feature_request(entity_id, as_of, &features)?;
        Ok(self.materialize(entity_id, as_of, features, tags))
    }

    #[must_use]
    pub fn latest(&self, entity_id: &str) -> Option<&FeatureSnapshot> {
        self.snapshots
            .iter()
            .rev()
            .find(|snapshot| snapshot.entity_id == entity_id)
    }
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_feature_request(
    entity_id: &str,
    as_of: &str,
    features: &BTreeMap<String, f64>,
) -> Result<(), FeatureError> {
    if !is_entity_id(entity_id) {
        return Err(FeatureError::InvalidEntityId);
    }
    if !is_date(as_of) {
        return Err(FeatureError::InvalidAsOf);
    }
    for (name, value) in features {
        if !is_feature_name(name) {
            return Err(FeatureError::InvalidFeatureName);
        }
        if !value.is_finite() {
            return Err(FeatureError::NonFiniteFeatureValue);
        }
    }
    Ok(())
}

fn is_entity_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
}

fn is_feature_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn is_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| matches!(index, 4 | 7) || character.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_tags::{TagAssignment, TagDefinition};

    #[test]
    fn materializes_feature_snapshot_with_active_tags() {
        let mut tags = TagStore::default();
        tags.define(TagDefinition {
            tag_id: "asset:equity".to_string(),
            parent: None,
            ttl_days: None,
        })
        .unwrap_or_else(|error| panic!("tag should define: {error}"));
        tags.assign(TagAssignment {
            entity_id: "instrument:AAPL".to_string(),
            tag_id: "asset:equity".to_string(),
            assigned_at: "2026-05-21".to_string(),
            expires_at: None,
            provenance: "manual".to_string(),
        })
        .unwrap_or_else(|error| panic!("tag should assign: {error}"));
        let mut features = BTreeMap::new();
        features.insert("return_1d".to_string(), 0.01);
        let mut store = FeatureStore::default();
        let snapshot = store.materialize("instrument:AAPL", "2026-05-21", features, &tags);

        assert_eq!(snapshot.tags, vec!["asset:equity".to_string()]);
        assert_eq!(
            store
                .latest("instrument:AAPL")
                .map(|latest| latest.as_of.clone()),
            Some("2026-05-21".to_string())
        );
    }

    #[test]
    fn checked_materialize_rejects_non_finite_features() {
        let tags = TagStore::default();
        let mut features = BTreeMap::new();
        features.insert("return_1d".to_string(), f64::NAN);
        let mut store = FeatureStore::default();

        assert_eq!(
            store.try_materialize("instrument:AAPL", "2026-05-21", features, &tags),
            Err(FeatureError::NonFiniteFeatureValue)
        );
    }

    #[test]
    fn validate_rejects_bad_entity_date_and_feature_name() {
        let mut feature = BTreeMap::new();
        feature.insert("return_1d".to_string(), 0.01);

        // Entity id with a space is invalid.
        assert_eq!(
            validate_feature_request("bad id", "2026-05-21", &feature),
            Err(FeatureError::InvalidEntityId)
        );
        // as_of must be a 10-char YYYY-MM-DD with '-' separators.
        assert_eq!(
            validate_feature_request("instrument:AAPL", "2026/05/21", &feature),
            Err(FeatureError::InvalidAsOf)
        );
        // Feature names allow only [alnum_-]; a '.' is rejected.
        let mut dotted = BTreeMap::new();
        dotted.insert("return.1d".to_string(), 0.01);
        assert_eq!(
            validate_feature_request("instrument:AAPL", "2026-05-21", &dotted),
            Err(FeatureError::InvalidFeatureName)
        );

        // A clean request validates.
        assert_eq!(
            validate_feature_request("instrument:AAPL", "2026-05-21", &feature),
            Ok(())
        );
    }
}
