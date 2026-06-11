//! End-to-end retrieval evaluation (knowledge-system B11): run a fixed case
//! set through a real vector+lexical `Retriever` over in-memory engines and
//! assert the aggregate metrics plus the leakage-safety property.

use std::sync::Arc;

use serde_json::json;
use tdw_core::{LexicalDoc, LexicalEngine, VectorEngine, VectorPoint};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_retrieve::Retriever;
use tdw_retrieve::evals::{DriftKey, RetrievalEvalCase, run_eval};
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;

const COLLECTION: &str = "tdw_knowledge_eval";
const INDEX: &str = "tdw_knowledge_text_eval";

/// Corpus: three dated documents. doc-old (2026-01-01) and doc-new
/// (2026-03-01) both match "earnings"; doc-mid (2026-02-01) matches "supply".
async fn retriever() -> Retriever {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let lexical = Arc::new(InMemoryLexicalEngine::default());

    let docs: [(&str, &str, &str); 3] = [
        (
            "doc-old",
            "acme earnings report strong cloud growth",
            "2026-01-01",
        ),
        ("doc-mid", "beta supply chain components note", "2026-02-01"),
        ("doc-new", "acme earnings beat later quarter", "2026-03-01"),
    ];
    let mut points = Vec::new();
    let mut lexical_docs = Vec::new();
    for (id, body, as_of) in docs {
        let payload = json!({
            "entity_id": format!("instrument:{id}"),
            "tags": [],
            "entity_kind": "instrument",
            "plane": "platform",
            "as_of": format!("{as_of}T00:00:00Z"),
        });
        let embedding = embedder.embed(body).await.expect("embed");
        points.push(VectorPoint {
            id: id.to_string(),
            vector: embedding.vector,
            payload: payload.clone(),
        });
        lexical_docs.push(LexicalDoc {
            id: id.to_string(),
            body: body.to_string(),
            fields: payload,
        });
    }
    vectors
        .upsert(COLLECTION, points)
        .await
        .expect("vector upsert");
    lexical
        .index(INDEX, lexical_docs)
        .await
        .expect("lexical index");

    Retriever::new(embedder, vectors, COLLECTION).with_lexical(lexical, INDEX)
}

fn drift_key() -> DriftKey {
    DriftKey {
        embedder_model: "local-hash-8".to_string(),
        rules_version: Some(1),
        infer_version: Some(1),
    }
}

#[tokio::test]
async fn run_eval_scores_a_fixed_case_set_deterministically() {
    let retriever = retriever().await;
    let cases = vec![
        RetrievalEvalCase {
            case_id: "earnings".to_string(),
            query: "acme earnings report strong cloud growth".to_string(),
            expected_doc_ids: vec!["doc-old".to_string()],
            as_of: None,
            tags_any: Vec::new(),
            fixed_split_id: Some("split-1".to_string()),
        },
        RetrievalEvalCase {
            case_id: "supply".to_string(),
            query: "beta supply chain components note".to_string(),
            expected_doc_ids: vec!["doc-mid".to_string()],
            as_of: None,
            tags_any: Vec::new(),
            fixed_split_id: Some("split-1".to_string()),
        },
    ];

    let report = run_eval(&retriever, &cases, 3, drift_key())
        .await
        .expect("eval runs");
    assert_eq!(report.k, 3);
    assert_eq!(report.cases.len(), 2);
    // Each query's exact-body doc is its top relevant hit.
    assert!(
        (report.mean_recall_at_k - 1.0).abs() < 1e-12,
        "both gold docs recalled: {}",
        report.mean_recall_at_k
    );
    assert!(report.mrr > 0.0 && report.mrr <= 1.0, "mrr {}", report.mrr);
    assert_eq!(report.drift_key, drift_key());

    // Determinism: a second run yields byte-identical metrics.
    let again = run_eval(&retriever, &cases, 3, drift_key())
        .await
        .expect("eval re-runs");
    assert_eq!(report, again);
}

#[tokio::test]
async fn eval_is_leakage_safe_by_as_of() {
    let retriever = retriever().await;
    // As of 2026-02-15, doc-new (2026-03-01) does NOT exist yet. A case that
    // (wrongly) expected the later doc must score ZERO recall — the retriever
    // structurally hides it, so the harness cannot leak future relevance.
    let cases = vec![RetrievalEvalCase {
        case_id: "temporal".to_string(),
        query: "acme earnings".to_string(),
        expected_doc_ids: vec!["doc-new".to_string()],
        as_of: Some("2026-02-15".to_string()),
        tags_any: Vec::new(),
        fixed_split_id: None,
    }];
    let report = run_eval(&retriever, &cases, 5, drift_key())
        .await
        .expect("eval runs");
    assert!(
        report.cases[0].recall_at_k.abs() < 1e-12,
        "future doc must be invisible under as_of: ranked={:?}",
        report.cases[0].ranked
    );
    assert!(
        !report.cases[0].ranked.iter().any(|id| id == "doc-new"),
        "doc-new leaked into a pre-publication query: {:?}",
        report.cases[0].ranked
    );
}

#[tokio::test]
async fn report_round_trips_through_json() {
    let retriever = retriever().await;
    let cases = vec![RetrievalEvalCase {
        case_id: "rt".to_string(),
        query: "acme earnings report strong cloud growth".to_string(),
        expected_doc_ids: vec!["doc-old".to_string()],
        as_of: None,
        tags_any: Vec::new(),
        fixed_split_id: None,
    }];
    let report = run_eval(&retriever, &cases, 3, drift_key())
        .await
        .expect("eval runs");
    let json = serde_json::to_string(&report).expect("serialize");
    let restored: tdw_retrieve::evals::RetrievalEvalReport =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(report, restored);
}
