//! Offline `tdw-feature-store` example: materialize a point-in-time feature
//! snapshot stamped with an entity's active tags.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-feature-store --example tdw-feature-store-basic
//! ```

use std::collections::BTreeMap;

use tdw_feature_store::FeatureStore;
use tdw_tags::{TagAssignment, TagDefinition, TagStore};

fn main() {
    // A tag store with one active tag on the instrument.
    let mut tags = TagStore::default();
    tags.define(TagDefinition {
        tag_id: "asset:equity".to_string(),
        parent: None,
        ttl_days: None,
    })
    .expect("tag should define");
    tags.assign(TagAssignment {
        entity_id: "instrument:AAPL".to_string(),
        tag_id: "asset:equity".to_string(),
        assigned_at: "2026-05-21".to_string(),
        expires_at: None,
        provenance: "manual".to_string(),
    })
    .expect("tag should assign");

    // Inline feature values for the snapshot.
    let mut features = BTreeMap::new();
    features.insert("return_1d".to_string(), 0.01);
    features.insert("volatility_20d".to_string(), 0.18);

    // Meaningful operation: materialize a validated snapshot, tags stamped from
    // the tag store as of the same date.
    let mut store = FeatureStore::default();
    let snapshot = store
        .try_materialize("instrument:AAPL", "2026-05-21", features, &tags)
        .expect("request should validate");

    println!(
        "snapshot {} as_of {} with {} feature(s), tags={:?}",
        snapshot.entity_id,
        snapshot.as_of,
        snapshot.features.len(),
        snapshot.tags
    );
    println!(
        "latest as_of: {:?}",
        store.latest("instrument:AAPL").map(|s| s.as_of.clone())
    );
}
