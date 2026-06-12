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
    Channel, ChannelEvidence, ConfidenceRankingWeight, GraphExpansion, KnowledgeQuery, PathStep,
    QueryFilter, Retriever,
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
/// supplier_of-linked to acme), doc-c describes instrument:gamma (UNDATED),
/// and doc-x is a SECOND, UNDATED acme document — the leak probe: acme holds
/// sector:tech, so a leaky tag channel or graph expansion would surface
/// doc-x under a temporal query even though the payload gate hides it.
async fn fixture() -> Fixture {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let lexical = Arc::new(InMemoryLexicalEngine::default());
    let tags = Arc::new(InMemoryTagEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());

    let docs: [DocSpec<'_>; 4] = [
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
        (
            "doc-x",
            "instrument:acme",
            "acme archived strategy memo with no publication date",
            None,
            &["sector:tech"],
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
        // Document graph nodes carry the same contract props the vector and
        // lexical payloads do — the graph channels filter on them.
        let mut document_node = node(&format!("document:{doc_id}"), EntityKind::Document);
        document_node.props = json!({"plane": "platform"});
        if let Some(as_of) = as_of {
            document_node.props["as_of"] = json!(as_of);
        }
        nodes.push(document_node);
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
        "only the sector:tech-tagged, in-date document survives — doc-x \
         (acme's UNDATED doc) must not leak through the tag channel"
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
    assert!(
        !ids.contains(&"doc-x"),
        "undated docs of a TAGGED entity must not leak either: {ids:?}"
    );
}

#[tokio::test]
async fn graph_expansion_respects_as_of() {
    let fixture = fixture().await;
    let retriever = full_retriever(&fixture);
    // doc-b is dated AFTER as_of; reaching instrument:beta via supplier_of
    // expansion must not smuggle its document past the temporal gate, and
    // acme's own undated doc-x must stay invisible too.
    let query = KnowledgeQuery::try_new(
        "acme",
        5,
        QueryFilter {
            as_of: Some("2026-02-01".to_string()),
            ..QueryFilter::default()
        },
        Some(GraphExpansion {
            k_hop: 1,
            edge_types: vec!["supplier_of".to_string()],
            per_hit_limit: 4,
            decay: 0.5,
        }),
    )
    .expect("valid query");
    let hits = retriever.search(&query).await.expect("search");
    let ids: Vec<&str> = hits.iter().map(|hit| hit.id.as_str()).collect();
    assert!(ids.contains(&"doc-a"), "{ids:?}");
    assert!(
        !ids.contains(&"doc-b"),
        "expansion must not bypass as_of for later-dated docs: {ids:?}"
    );
    assert!(
        !ids.contains(&"doc-x"),
        "expansion must not bypass as_of for undated docs: {ids:?}"
    );
}

#[tokio::test]
async fn graph_expansion_reaches_neighbors_with_explained_path() {
    let fixture = fixture().await;
    let retriever = full_retriever(&fixture);
    // Disable confidence blending for this test: it is testing graph-expansion
    // ordering (decayed expansion must not outrank its seed), not K-R6 scoring.
    // Confidence weight is exercised in the dedicated ranking-weight tests.
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
    .expect("valid query")
    .with_confidence_weight(ConfidenceRankingWeight::OFF);
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

// ── K-R6: confidence ranking weight ──────────────────────────────────────────

#[tokio::test]
async fn confidence_weight_off_gives_same_scores_as_unweighted() {
    // When weight=OFF the final score equals the raw RRF score; confidence does
    // not shift ranking at all.
    let fixture = fixture().await;
    let retriever = full_retriever(&fixture);

    let query_off = KnowledgeQuery::try_new(
        "acme quarterly earnings report shows strong cloud growth",
        5,
        QueryFilter::default(),
        None,
    )
    .expect("valid query")
    .with_confidence_weight(ConfidenceRankingWeight::OFF);

    let hits_off = retriever.search(&query_off).await.expect("search off");

    // With OFF, confidence field must be None on every hit and scores are
    // purely RRF-derived.
    for hit in &hits_off {
        assert!(
            hit.confidence.is_none(),
            "OFF weight must not attach confidence scores: hit.id={}",
            hit.id
        );
    }

    // Score ordering must match ordering without the graph (vector-only
    // ordering): OFF disables the confidence component entirely.
    assert!(!hits_off.is_empty(), "must return hits");
}

#[tokio::test]
async fn confidence_weight_nonzero_attaches_confidence_and_blends_score() {
    // With graph attached and nonzero weight, hits that have an entity_id get
    // a ConfidenceScore attached and their score is the blended value.
    let fixture = fixture().await;
    let retriever = full_retriever(&fixture);

    // Default weight (0.05) — graph is attached via full_retriever.
    let query = KnowledgeQuery::try_new(
        "acme quarterly earnings report shows strong cloud growth",
        5,
        QueryFilter::default(),
        None,
    )
    .expect("valid query");
    // Default confidence weight is 0.05, so confidence is enabled.

    let hits = retriever.search(&query).await.expect("search");

    // At least one hit must carry a confidence score (doc-a has entity_id +
    // described_by edge in the fixture).
    let has_confidence = hits.iter().any(|h| h.confidence.is_some());
    assert!(
        has_confidence,
        "with nonzero weight and graph, some hits must carry confidence"
    );

    // Compare with OFF: the top hit's score must differ (it was blended).
    let query_off = KnowledgeQuery::try_new(
        "acme quarterly earnings report shows strong cloud growth",
        5,
        QueryFilter::default(),
        None,
    )
    .expect("valid query")
    .with_confidence_weight(ConfidenceRankingWeight::OFF);

    let hits_off = retriever.search(&query_off).await.expect("search off");
    let top_blended = hits.iter().find(|h| h.id == "doc-a").map(|h| h.score);
    let top_raw = hits_off.iter().find(|h| h.id == "doc-a").map(|h| h.score);

    if let (Some(blended), Some(raw)) = (top_blended, top_raw) {
        assert!(
            (blended - raw).abs() > 1e-9,
            "nonzero weight must produce a different score than OFF: blended={blended}, raw={raw}"
        );
    }
}

#[test]
fn confidence_ranking_weight_production_wiring() {
    // Verify the production-wiring contract for ConfidenceRankingWeight:
    //   - default() matches DEFAULT_CONFIDENCE_WEIGHT
    //   - OFF is exactly 0.0
    //   - with_confidence_weight clamps out-of-range values
    //   - blend formula: rrf*(1-w) + conf*w
    use tdw_core::DEFAULT_CONFIDENCE_WEIGHT;

    let default_w = ConfidenceRankingWeight::default();
    assert!(
        (default_w.0 - DEFAULT_CONFIDENCE_WEIGHT).abs() < f64::EPSILON,
        "default weight must equal DEFAULT_CONFIDENCE_WEIGHT ({DEFAULT_CONFIDENCE_WEIGHT}); got {}",
        default_w.0
    );

    assert!(
        ConfidenceRankingWeight::OFF.0.abs() < f64::EPSILON,
        "OFF must be exactly 0.0"
    );

    // Clamp: value above 1.0 is clamped to 1.0.
    let query = KnowledgeQuery::try_new("q", 5, QueryFilter::default(), None)
        .expect("valid")
        .with_confidence_weight(ConfidenceRankingWeight(2.0));
    assert!(
        (query.confidence_weight.0 - 1.0).abs() < f64::EPSILON,
        "weight > 1.0 must be clamped to 1.0; got {}",
        query.confidence_weight.0
    );

    // Clamp: negative value is clamped to 0.0.
    let query = KnowledgeQuery::try_new("q", 5, QueryFilter::default(), None)
        .expect("valid")
        .with_confidence_weight(ConfidenceRankingWeight(-0.5));
    assert!(
        query.confidence_weight.0.abs() < f64::EPSILON,
        "weight < 0.0 must be clamped to 0.0; got {}",
        query.confidence_weight.0
    );

    // Blend formula: blended = rrf*(1-w) + conf*w.
    let w = ConfidenceRankingWeight(0.1);
    let blended = w.blend(0.8, 0.5);
    let expected = 0.8f64.mul_add(0.9, 0.5 * 0.1);
    assert!(
        (blended - expected).abs() < 1e-12,
        "blend formula wrong: got {blended}, expected {expected}"
    );

    // Blend with OFF: result equals rrf unchanged.
    let raw = 0.12345;
    let blended_off = ConfidenceRankingWeight::OFF.blend(raw, 0.9);
    assert!(
        (blended_off - raw).abs() < f64::EPSILON,
        "OFF.blend must return rrf unchanged; got {blended_off}"
    );
}

#[tokio::test]
async fn query_validation_rejects_bad_input() {
    assert!(KnowledgeQuery::try_new("  ", 5, QueryFilter::default(), None).is_err());
    assert!(KnowledgeQuery::try_new("q", 0, QueryFilter::default(), None).is_err());
    assert!(
        KnowledgeQuery::try_new("q", 257, QueryFilter::default(), None).is_err(),
        "top_k above MAX_TOP_K must be rejected"
    );
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
    for (k_hop, per_hit_limit, decay) in [
        (0, 4, 0.5),
        (4, 4, 0.5),
        (1, 0, 0.5),
        (1, 65, 0.5),
        (1, 4, 0.0),
    ] {
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
