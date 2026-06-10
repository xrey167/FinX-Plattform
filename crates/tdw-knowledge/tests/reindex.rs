//! Embedder-switch reindex (knowledge-system B6): rebuild a NEW embedder's
//! collection from the durable lexical documents, payload carried verbatim.

use std::sync::Arc;

use tdw_core::{LexicalEngine, VectorEngine, VectorQuery};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_kg::{Entity, EntityKind};
use tdw_knowledge::indexer::KnowledgeIndexer;
use tdw_knowledge::reindex::reindex_collection;
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex, collection_name};
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;

const TEXT_INDEX: &str = "tdw_knowledge_text_test";

fn doc(id: &str, body: &str, as_of: Option<&str>) -> KnowledgeDocument {
    let mut document = KnowledgeDocument::new(
        id,
        body,
        Entity {
            entity_id: format!("instrument:{}", id.to_ascii_uppercase()),
            kind: EntityKind::Instrument,
            label: id.to_string(),
            aliases: Vec::new(),
        },
        vec!["asset:equity".to_string()],
    );
    document.plane = Some("platform".to_string());
    document.as_of = as_of.map(ToString::to_string);
    document
}

#[tokio::test]
async fn reindex_rebuilds_a_new_embedder_collection_with_payload_intact() {
    let vectors: Arc<InMemoryVectorEngine> = Arc::new(InMemoryVectorEngine::default());
    let lexical: Arc<InMemoryLexicalEngine> = Arc::new(InMemoryLexicalEngine::default());

    // Ingest with the DEFAULT embedder (8-dim hash) + lexical co-index.
    let mut indexer = KnowledgeIndexer::new(KnowledgeIndex::new(
        Arc::new(HashEmbeddingProvider::default()),
        vectors.clone(),
    ))
    .with_lexical(lexical.clone(), TEXT_INDEX);
    for (id, body, as_of) in [
        ("doc-a", "acme earnings beat", Some("2026-03-05")),
        ("doc-b", "beta supply note", Some("2026-04-01")),
        ("doc-c", "gamma undated memo", None),
    ] {
        indexer
            .index_at(doc(id, body, as_of), "2026-06-01")
            .await
            .expect("ingest");
    }

    // Switch: a 16-dim hash embedder under a NEW model id.
    let target: Arc<dyn EmbeddingProvider> =
        Arc::new(HashEmbeddingProvider::new("local-hash-16", 16).expect("valid embedder"));
    let vector_engine: Arc<dyn VectorEngine> = vectors.clone();
    let lexical_engine: Arc<dyn LexicalEngine> = lexical.clone();
    let count = reindex_collection(&target, &vector_engine, &lexical_engine, TEXT_INDEX)
        .await
        .expect("reindex");
    assert_eq!(count, 3);

    // The new collection answers searches with 16-dim vectors and the FULL
    // payload contract carried over (entity_id, tags, plane, as_of).
    let query_vector = target
        .embed("acme earnings beat")
        .await
        .expect("query embed")
        .vector;
    assert_eq!(query_vector.len(), 16);
    let hits = vectors
        .search_knn(
            &collection_name("local-hash-16"),
            VectorQuery::knn(query_vector, 3),
        )
        .await
        .expect("search new collection");
    assert_eq!(hits.len(), 3);
    let acme = hits
        .iter()
        .find(|hit| hit.id == "doc-a")
        .expect("doc-a present");
    assert_eq!(acme.payload["entity_id"], "instrument:DOC-A");
    assert_eq!(acme.payload["plane"], "platform");
    assert_eq!(acme.payload["as_of"], "2026-03-05T00:00:00Z");
    let gamma = hits
        .iter()
        .find(|hit| hit.id == "doc-c")
        .expect("doc-c present");
    assert!(
        gamma.payload.get("as_of").is_none(),
        "undated stays undated through a reindex"
    );

    // Re-running is idempotent (upsert by id).
    let again = reindex_collection(&target, &vector_engine, &lexical_engine, TEXT_INDEX)
        .await
        .expect("re-run");
    assert_eq!(again, 3);

    // The OLD collection is untouched.
    let old_vector = HashEmbeddingProvider::default()
        .embed("acme earnings beat")
        .await
        .expect("old embed")
        .vector;
    let old_hits = vectors
        .search_knn(
            &collection_name("local-hash-8"),
            VectorQuery::knn(old_vector, 3),
        )
        .await
        .expect("old collection still answers");
    assert_eq!(old_hits.len(), 3);
}

#[tokio::test]
async fn lexical_documents_scan_paginates_without_gaps_or_overlap() {
    let lexical = InMemoryLexicalEngine::default();
    let docs: Vec<tdw_core::LexicalDoc> = (0..5)
        .map(|index| tdw_core::LexicalDoc {
            id: format!("doc-{index}"),
            body: format!("body {index}"),
            fields: serde_json::json!({"entity_id": format!("e{index}")}),
        })
        .collect();
    lexical.index(TEXT_INDEX, docs).await.expect("index");

    let mut seen = Vec::new();
    let mut offset = 0;
    loop {
        let page = lexical
            .documents(TEXT_INDEX, offset, 2)
            .await
            .expect("page");
        if page.is_empty() {
            break;
        }
        offset += page.len();
        seen.extend(page.into_iter().map(|doc| doc.id));
    }
    seen.sort();
    assert_eq!(seen, vec!["doc-0", "doc-1", "doc-2", "doc-3", "doc-4"]);

    assert!(
        lexical.documents(TEXT_INDEX, 0, 0).await.is_err(),
        "zero limit is rejected"
    );
    assert!(
        lexical.documents("missing-index", 0, 2).await.is_err(),
        "unknown index errors loudly"
    );
}
