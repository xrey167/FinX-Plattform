//! Offline `tdw-knowledge` example: index a document and search it back through
//! the default offline index (hash embedder + in-process vector engine).
//! Deterministic, no network.
//!
//! ```text
//! cargo run --example tdw_knowledge_basic -p tdw-knowledge
//! ```

use tdw_kg::{Entity, EntityKind};
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex};

#[tokio::main]
async fn main() {
    let mut index = KnowledgeIndex::default();

    index
        .index_document(KnowledgeDocument {
            id: "doc-1".to_string(),
            body: "AAPL equity momentum research".to_string(),
            entity: Entity {
                entity_id: "instrument:AAPL".to_string(),
                kind: EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: vec!["AAPL".to_string()],
            },
            tags: vec!["asset:equity".to_string()],
        })
        .await
        .expect("indexing should succeed");

    let hits = index
        .search("AAPL momentum", 1)
        .await
        .expect("search should succeed");

    assert_eq!(hits[0].id, "doc-1");
    assert_eq!(hits[0].entity_id, "instrument:AAPL");
    println!("top hit: {} (entity {})", hits[0].id, hits[0].entity_id);

    // The entity's tags are active as of the indexing date.
    let tags = index.active_tags("instrument:AAPL", "2026-05-22");
    assert_eq!(tags, vec!["asset:equity".to_string()]);
    println!("active tags: {tags:?}");
}
