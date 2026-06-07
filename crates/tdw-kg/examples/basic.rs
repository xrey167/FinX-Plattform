//! Offline `tdw-kg` example: build an in-memory knowledge graph, query
//! neighbors, record an audited manual merge, and show the checked-path
//! rejections. Fully in-process — no async, no I/O, no network.
//!
//! ```text
//! cargo run --example tdw_kg_basic -p tdw-kg
//! ```

use tdw_kg::{Entity, EntityKind, KnowledgeGraph, KnowledgeGraphError, Relationship};

fn main() {
    let mut kg = KnowledgeGraph::default();

    kg.upsert_entity(Entity {
        entity_id: "instrument:AAPL".to_string(),
        kind: EntityKind::Instrument,
        label: "Apple".to_string(),
        aliases: vec!["AAPL".to_string()],
    });
    kg.upsert_entity(Entity {
        entity_id: "dataset:ohlcv".to_string(),
        kind: EntityKind::Dataset,
        label: "OHLCV".to_string(),
        aliases: Vec::new(),
    });
    kg.add_relationship(Relationship {
        from: "instrument:AAPL".to_string(),
        to: "dataset:ohlcv".to_string(),
        rel_type: "has_prices".to_string(),
        provenance: "fixture".to_string(),
    });

    let neighbors = kg.neighbors("instrument:AAPL");
    assert_eq!(neighbors[0].entity_id, "dataset:ohlcv");
    println!("neighbors of instrument:AAPL -> {}", neighbors[0].entity_id);

    // An audited manual merge requires an approver.
    assert!(kg.manual_merge("instrument:AAPL", "dataset:ohlcv", "architect"));
    assert_eq!(kg.merge_audit().len(), 1);
    println!("merge audit: {}", kg.merge_audit()[0]);

    // Checked paths reject a traversal-style id and a dangling edge.
    assert_eq!(
        kg.try_upsert_entity(Entity {
            entity_id: "../instrument".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: Vec::new(),
        }),
        Err(KnowledgeGraphError::InvalidEntityId)
    );
    assert_eq!(
        kg.try_add_relationship(Relationship {
            from: "instrument:AAPL".to_string(),
            to: "dataset:missing".to_string(),
            rel_type: "has_prices".to_string(),
            provenance: "fixture".to_string(),
        }),
        Err(KnowledgeGraphError::MissingEndpoint)
    );
    println!("invalid id and dangling edge are rejected, as expected");
}
