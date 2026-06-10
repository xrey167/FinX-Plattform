//! End-to-end integration test: real Qdrant vector backend durability.
//!
//! Proves that knowledge indexed through a [`KnowledgeIndex`] built over the
//! daemon's shared `AppState.vector` (a real `QdrantHttpEngine`) PERSISTS: the
//! index is dropped and a FRESH `KnowledgeIndex` over a NEW `QdrantHttpEngine`
//! to the same URL still finds the document.
//!
//! # Gating
//! - Feature gate: compiled only with `--features real-qdrant` (activates
//!   `tdw-storage-qdrant/qdrant` and exposes `with_real_qdrant`).
//! - Runtime gate: reads `TDW_QDRANT_TEST_URL`; if unset the test returns
//!   immediately (exit 0) so offline CI still passes.
//!
//! # Running locally
//! ```sh
//! # Qdrant up (HTTP interface on 6333)
//! export TDW_QDRANT_TEST_URL="http://127.0.0.1:6333"
//! cargo test -p tdw-service-api --features real-qdrant \
//!   --test qdrant_knowledge_durability_e2e -- --nocapture
//! ```
//!
//! Optional env vars:
//!   - `TDW_QDRANT_TEST_API_KEY` (default: unset)
#![cfg(feature = "real-qdrant")]

use std::sync::Arc;

use tdw_embed_local::HashEmbeddingProvider;
use tdw_kg::{Entity, EntityKind};
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex};
use tdw_service_api::AppState;
use tdw_storage_qdrant::QdrantHttpEngine;

#[tokio::test]
async fn e2e_real_qdrant_knowledge_persists_across_index_instances() {
    let Ok(url) = std::env::var("TDW_QDRANT_TEST_URL") else {
        eprintln!("skipping qdrant durability e2e: TDW_QDRANT_TEST_URL not set");
        return;
    };
    let api_key = std::env::var("TDW_QDRANT_TEST_API_KEY").ok();

    // Build AppState with in-memory stores but a REAL Qdrant vector engine, then
    // index a document through a KnowledgeIndex riding on that shared engine.
    let state = AppState::in_memory_for_tests()
        .await
        .with_real_qdrant(&url, api_key.clone())
        .expect("QdrantHttpEngine build failed — check TDW_QDRANT_TEST_URL");

    let document = KnowledgeDocument {
        id: "doc-durable".to_string(),
        body: "AAPL equity momentum research".to_string(),
        entity: Entity {
            entity_id: "instrument:AAPL".to_string(),
            kind: EntityKind::Instrument,
            label: "Apple".to_string(),
            aliases: vec!["AAPL".to_string()],
        },
        tags: vec!["asset:equity".to_string()],
        source: None,
        plane: None,
        as_of: None,
        mentions: Vec::new(),
    };

    {
        let mut index = KnowledgeIndex::new(
            Arc::new(HashEmbeddingProvider::default()),
            state.vector.clone(),
        );
        index
            .index_document_at(document, "2026-05-22")
            .await
            .unwrap_or_else(|error| panic!("index against live Qdrant must succeed: {error}"));
    } // index (and its Arc clone of the engine) dropped here.

    // A FRESH KnowledgeIndex over a NEW QdrantHttpEngine to the same URL must
    // still find the document — proving it was persisted server-side, not held
    // in process.
    let fresh_engine = QdrantHttpEngine::new(&url, api_key)
        .expect("fresh QdrantHttpEngine build failed — check TDW_QDRANT_TEST_URL");
    let fresh_index = KnowledgeIndex::new(
        Arc::new(HashEmbeddingProvider::default()),
        Arc::new(fresh_engine),
    );

    let hits = fresh_index
        .search("AAPL momentum", 1)
        .await
        .unwrap_or_else(|error| panic!("search against live Qdrant must succeed: {error}"));

    assert!(
        !hits.is_empty(),
        "fresh index over the same Qdrant must return the persisted document"
    );
    assert_eq!(hits[0].id, "doc-durable");
    assert_eq!(hits[0].entity_id, "instrument:AAPL");
}
