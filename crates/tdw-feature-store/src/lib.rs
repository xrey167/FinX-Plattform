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

    pub fn latest(&self, entity_id: &str) -> Option<&FeatureSnapshot> {
        self.snapshots
            .iter()
            .rev()
            .find(|snapshot| snapshot.entity_id == entity_id)
    }
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
}
