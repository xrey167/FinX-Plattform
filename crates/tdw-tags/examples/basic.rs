//! Offline `tdw-tags` example: define a small taxonomy, assign a time-bounded
//! tag to an instrument, and query active tags as of two dates.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-tags --example tdw-tags-basic
//! ```

use tdw_tags::{TagAssignment, TagDefinition, TagStore};

fn main() {
    let mut store = TagStore::default();

    // Build a tiny parent/child taxonomy.
    store
        .define(TagDefinition {
            tag_id: "asset:equity".to_string(),
            parent: None,
            ttl_days: None,
        })
        .expect("root tag should define");
    store
        .define(TagDefinition {
            tag_id: "style:momentum".to_string(),
            parent: Some("asset:equity".to_string()),
            ttl_days: Some(30),
        })
        .expect("child tag should define");

    // Assign a tag over a [assigned_at, expires_at) window.
    store
        .assign(TagAssignment {
            entity_id: "instrument:AAPL".to_string(),
            tag_id: "style:momentum".to_string(),
            assigned_at: "2026-05-21".to_string(),
            expires_at: Some("2026-06-20".to_string()),
            provenance: "rule:price_momentum".to_string(),
        })
        .expect("assignment should persist");

    // Meaningful operation: point-in-time active-tag queries.
    println!(
        "active on 2026-05-22: {:?}",
        store.active_tags("instrument:AAPL", "2026-05-22")
    );
    println!(
        "active on 2026-07-01 (after expiry): {:?}",
        store.active_tags("instrument:AAPL", "2026-07-01")
    );
    println!("taxonomy stats: {:?}", store.taxonomy_stats());
}
