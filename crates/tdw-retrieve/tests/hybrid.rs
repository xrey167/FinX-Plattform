//! End-to-end hybrid retrieval over the in-memory reference engines
//! (knowledge-system B4): vector + lexical + tag channels fused with RRF,
//! temporal `as_of` filtering, and explained graph expansion.

use std::sync::Arc;

use serde_json::json;
use tdw_core::{
    GraphEdge, GraphEngine, GraphNode, LexicalDoc, LexicalEngine, PayloadFilter, Provenance,
    VectorEngine, VectorPoint, VectorQuery,
};
use tdw_embed::EmbeddingProvider;
use tdw_embed_local::HashEmbeddingProvider;
use tdw_retrieve::{
    Channel, ChannelEvidence, GraphExpansion, KnowledgeQuery, PathStep, QueryFilter, Retriever,
};
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_tags::{InMemoryTagEngine, TagAssignment, TagDefinition, TagEngine};
use tdw_taxonomy::EntityKind;

const COLLECTION: &str = "tdw_knowledge_test";
const INDEX: &str = "tdw_knowledge_text_test";

struct Fixture {
    embedder: Arc<HashEmbeddingProvider>,
    vectors: Arc<InMemoryVectorEngine>,
    lexical: Arc<InMemoryLexicalEngine>,
    tags: Arc<InMemoryTagEngine>,
    graph: Arc<InMemoryGraphEngine>,
}

/// One fixture document: `(doc_id, entity_id, body, as_of, tags)`.
type DocSpec<'a> = (&'a str, &'a str, &'a str, Option<&'a str>, &'a [&'a str]);

/// Corpus: doc-a describes instrument:acme (tagged sector:tech, dated
/// 2026-01-15), doc-b describes instrument:beta (dated 2026-03-01,
/// supplier_of-linked to acme), doc-c describes instrument:gamma (UNDATED).
async fn fixture() -> Fixture {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let lexical = Arc::new(InMemoryLexicalEngine::default());
    let tags = Arc::new(InMemoryTagEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());

    let docs: [DocSpec<'_>; 3] = [
        (
            "doc-a",
            "instrument:acme",
            "acme quarterly earnings report shows strong cloud growth",
            Some("2026-01-15T00:00:00Z"),
            &["sector:tech"],
        ),
        (
            "doc-b",
            "instrument:beta",
            "beta industries supplies components and reports earnings",
            Some("2026-03-01T00:00:00Z"),
            &[],
        ),
        (
            "doc-c",
            "instrument:gamma",
            "gamma holdings earnings note without a publication date",
            None,
            &[],
        ),
    ];

    let mut points = Vec::new();
    let mut lexical_docs = Vec::new();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (doc_id, entity_id, body, as_of, doc_tags) in docs {
        let mut payload = json!({
            "entity_id": entity_id,
            "tags": doc_tags,
            "entity_kind": "instrument",
            "plane": "platform",
        });
        if let Some(as_of) = as_of {
            payload["as_of"] = json!(as_of);
        }
        let embedding = embedder.embed(body).await.expect("embed fixture doc");
        points.push(VectorPoint {
            id: doc_id.to_string(),
            vector: embedding.vector,
            payload: payload.clone(),
        });
        lexical_docs.push(LexicalDoc {
            id: doc_id.to_string(),
            body: body.to_string(),
            fields: payload,
        });
        nodes.push(node(entity_id, EntityKind::Instrument));
        nodes.push(node(&format!("document:{doc_id}"), EntityKind::Document));
        edges.push(edge(
            entity_id,
            &format!("document:{doc_id}"),
            "described_by",
        ));
    }
    edges.push(edge("instrument:acme", "instrument:beta", "supplier_of"));

    vectors
        .upsert(COLLECTION, points)
        .await
        .expect("vector upsert");
    lexical
        .index(INDEX, lexical_docs)
        .await
        .expect("lexical index");
    graph.upsert_nodes(nodes).await.expect("graph nodes");
    graph.upsert_edges(edges).await.expect("graph edges");

    seed_taxonomy(tags.as_ref()).await;

    Fixture {
        embedder,
        vectors,
        lexical,
        tags,
        graph,
    }
}

/// Taxonomy `sector:all` -> `sector:tech`; acme holds `sector:tech` from
/// 2026-01-10, open-ended.
async fn seed_taxonomy(tags: &dyn TagEngine) {
    tags.define(TagDefinition {
        tag_id: "sector:all".to_string(),
        parent: None,
        ttl_days: None,
    })
    .await
    .expect("define sector:all");
    tags.define(TagDefinition {
        tag_id: "sector:tech".to_string(),
        parent: Some("sector:all".to_string()),
        ttl_days: None,
    })
    .await
    .expect("define sector:tech");
    tags.assign(TagAssignment {
        entity_id: "instrument:acme".to_string(),
        tag_id: "sector:tech".to_string(),
        assigned_at: "2026-01-10".to_string(),
        expires_at: None,
        provenance: "test:fixture".to_string(),
    })
    .await
    .expect("assign sector:tech");
}

fn node(id: &str, kind: EntityKind) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind,
        label: id.to_string(),
        aliases: Vec::new(),
        props: json!({}),
        valid_from: None,
        valid_to: None,
    }
}

fn edge(from: &str, to: &str, rel: &str) -> GraphEdge {
    GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
        props: json!({}),
        provenance: Provenance::Ingest {
            source: "test:fixture".to_string(),
        },
        valid_from: None,
        valid_to: None,
    }
}

fn full_retriever(fixture: &Fixture) -> Retriever {
    Retriever::new(
        fixture.embedder.clone(),
        fixture.vectors.clone(),
        COLLECTION,
    )
    .with_lexical(fixture.lexical.clone(), INDEX)
    .with_tags(fixture.tags.clone())
    .with_graph(fixture.graph.clone())
}

#[tokio::test]
async fn vector_only_retriever_preserves_raw_knn_order() {
    let fixture = fixture().await;
    let retriever = Retriever::new(
        fixture.embedder.clone(),
        fixture.vectors.clone(),
        COLLECTION,
    );
    let text = "acme quarterly earnings report shows strong cloud growth";
    let query =
        KnowledgeQuery::try_new(text, 2, QueryFilter::default(), None).expect("valid query");
    // The vector-only retriever is the pre-B4 KnowledgeIndex::search shape:
    // its order must be exactly the engine's KNN order.
    let embedding = fixture.embedder.embed(text).await.expect("embed query");
    let raw = fixture
        .vectors
        .search_knn(
            COLLECTION,
            VectorQuery {
                vector: embedding.vector,
                top_k: query.channel_top_k,
                filter: PayloadFilter::default(),
            },
        )
        .await
        .expect("raw knn");
    let hits = retriever.search(&query).await.expect("search");
    assert_eq!(hits.len(), 2);
    for (position, hit) in hits.iter().enumerate() {
        assert_eq!(hit.id, raw[position].id, "retriever must mirror raw KNN");
        assert_eq!(
            hit.explanation.channels,
            vec![ChannelEvidence {
                channel: Channel::Vector,
                rank: position + 1,
                raw_score: raw[position].score,
            }]
        );
    }
}

#[tokio::test]
async fn hybrid_fusion_combines_vector_lexical_and_tag_channels() {
    let fixture = fixture().await;
    let retriever = full_retriever(&fixture);
    // tags_any = ["sector:all"]: subsumption must expand to sector:tech, the
    // tag channel must surface acme's document, and the payload filter must
    // hide untagged docs from the vector/lexical channels. Query text is a
    // single word — the in-memory lexical engine is a substring matcher.
    let query = KnowledgeQuery::try_new(
        "acme",
        5,
        QueryFilter {
            tags_any: vec!["sector:all".to_string()],
            entity_kinds: Some(vec![EntityKind::Instrument]),
            plane: Some("platform".to_string()),
            as_of: Some("2026-02-01".to_string()),
        },
        None,
    )
    .expect("valid query");
    let hits = retriever.search(&query).await.expect("search");
    assert_eq!(
        hits.iter().map(|hit| hit.id.as_str()).collect::<Vec<_>>(),
        vec!["doc-a"],
        "only the sector:tech-tagged, in-date document survives"
    );
    let channels: Vec<Channel> = hits[0]
        .explanation
        .channels
        .iter()
        .map(|evidence| evidence.channel)
        .collect();
    assert!(channels.contains(&Channel::Vector), "{channels:?}");
    assert!(channels.contains(&Channel::Lexical), "{channels:?}");
    assert!(
        channels.contains(&Channel::Tag),
        "subsumption-expanded tag channel must contribute: {channels:?}"
    );
    assert_eq!(hits[0].explanation.matched_tags, vec!["sector:tech"]);
}

#[tokio::test]
async fn as_of_excludes_later_and_undated_documents() {
    let fixture = fixture().await;
    let retriever = full_retriever(&fixture);
    let query = KnowledgeQuery::try_new(
        "earnings",
        5,
        QueryFilter {
            as_of: Some("2026-02-01".to_string()),
            ..QueryFilter::default()
        },
        None,
    )
    .expect("valid query");
    let hits = retriever.search(&query).await.expect("search");
    let ids: Vec<&str> = hits.iter().map(|hit| hit.id.as_str()).collect();
    assert!(ids.contains(&"doc-a"), "{ids:?}");
    assert!(
        !ids.contains(&"doc-b"),
        "doc-b is dated AFTER as_of and must be invisible: {ids:?}"
    );
    assert!(
        !ids.contains(&"doc-c"),
        "undated docs are excluded by temporal queries: {ids:?}"
    );
}

#[tokio::test]
async fn graph_expansion_reaches_neighbors_with_explained_path() {
    let fixture = fixture().await;
    let retriever = full_retriever(&fixture);
    let query = KnowledgeQuery::try_new(
        "acme quarterly earnings report shows strong cloud growth",
        5,
        QueryFilter::default(),
        Some(GraphExpansion {
            k_hop: 1,
            edge_types: vec!["supplier_of".to_string()],
            per_hit_limit: 4,
            decay: 0.5,
        }),
    )
    .expect("valid query");
    let hits = retriever.search(&query).await.expect("search");
    let expanded = hits
        .iter()
        .find(|hit| hit.id == "doc-b")
        .expect("supplier_of expansion must reach beta's document");
    assert_eq!(expanded.entity_id, "instrument:beta");
    assert_eq!(expanded.explanation.seed_hit.as_deref(), Some("doc-a"));
    assert_eq!(
        expanded.explanation.graph_path,
        Some(vec![PathStep {
            from: "instrument:acme".to_string(),
            edge_type: "supplier_of".to_string(),
            to: "instrument:beta".to_string(),
        }])
    );
    let seed = hits.iter().find(|hit| hit.id == "doc-a").expect("seed hit");
    assert!(
        seed.score > expanded.score,
        "decayed expansion must not outrank its seed"
    );
}

#[tokio::test]
async fn query_validation_rejects_bad_input() {
    assert!(KnowledgeQuery::try_new("  ", 5, QueryFilter::default(), None).is_err());
    assert!(KnowledgeQuery::try_new("q", 0, QueryFilter::default(), None).is_err());
    assert!(
        KnowledgeQuery::try_new(
            "q",
            5,
            QueryFilter {
                as_of: Some("01.02.2026".to_string()),
                ..QueryFilter::default()
            },
            None,
        )
        .is_err(),
        "as_of must be YYYY-MM-DD"
    );
    for (k_hop, per_hit_limit, decay) in [(0, 4, 0.5), (4, 4, 0.5), (1, 0, 0.5), (1, 4, 0.0)] {
        assert!(
            KnowledgeQuery::try_new(
                "q",
                5,
                QueryFilter::default(),
                Some(GraphExpansion {
                    k_hop,
                    edge_types: Vec::new(),
                    per_hit_limit,
                    decay,
                }),
            )
            .is_err(),
            "expansion ({k_hop}, {per_hit_limit}, {decay}) must be rejected"
        );
    }
}
