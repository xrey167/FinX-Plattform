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
    TrustClass,
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
            provenance_classes: None,
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

// ── K-X3 Trust-dial tests ────────────────────────────────────────────────────

/// Build a minimal trust-dial fixture: three documents with different
/// `provenance_class` stamps in their payload, and one LEGACY document
/// (no `provenance_class` field at all — simulates a pre-stamp index point).
async fn trust_fixture() -> (Arc<HashEmbeddingProvider>, Arc<InMemoryVectorEngine>) {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());

    let docs: &[(&str, &str, &str)] = &[
        ("doc-ingested", "instrument:acme", "document_ingested"),
        ("doc-user", "finding:abc123", "user_authored"),
        ("doc-agent", "instrument:beta", "agent_proposed"),
        // legacy: no provenance_class field — treated as document_ingested
        ("doc-legacy", "instrument:gamma", ""),
    ];

    let mut points = Vec::new();
    for (doc_id, entity_id, class) in docs {
        let embedding = embedder.embed(doc_id).await.expect("embed");
        let mut payload = json!({
            "entity_id": entity_id,
            "tags": [],
            "entity_kind": "instrument",
        });
        if !class.is_empty() {
            payload["provenance_class"] = json!(class);
        }
        points.push(VectorPoint {
            id: doc_id.to_string(),
            vector: embedding.vector,
            payload,
        });
    }
    vectors
        .upsert(COLLECTION, points)
        .await
        .expect("vector upsert");
    (embedder, vectors)
}

fn trust_retriever(
    embedder: Arc<HashEmbeddingProvider>,
    vectors: Arc<InMemoryVectorEngine>,
) -> Retriever {
    Retriever::new(embedder, vectors, COLLECTION)
}

#[tokio::test]
async fn default_filter_returns_all_classes_unchanged() {
    // No provenance_classes filter → all four docs may surface (regression
    // safety: pre-K-X3 callers must see unchanged behaviour).
    let (embedder, vectors) = trust_fixture().await;
    let retriever = trust_retriever(embedder, vectors);
    let query =
        KnowledgeQuery::try_new("document", 16, QueryFilter::default(), None).expect("valid query");
    let hits = retriever.search(&query).await.expect("search");
    let ids: std::collections::BTreeSet<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert!(
        ids.contains("doc-ingested"),
        "ingested doc must appear: {ids:?}"
    );
    assert!(ids.contains("doc-user"), "user doc must appear: {ids:?}");
    assert!(ids.contains("doc-agent"), "agent doc must appear: {ids:?}");
    assert!(
        ids.contains("doc-legacy"),
        "legacy (no stamp) doc must appear: {ids:?}"
    );
    // Every hit that has a stamp carries its trust_class for explainability.
    for hit in &hits {
        if hit.id == "doc-ingested" {
            assert_eq!(
                hit.trust_class,
                Some(TrustClass::DocumentIngested),
                "doc-ingested must carry DocumentIngested trust_class"
            );
        }
        if hit.id == "doc-user" {
            assert_eq!(
                hit.trust_class,
                Some(TrustClass::UserAuthored),
                "doc-user must carry UserAuthored trust_class"
            );
        }
        if hit.id == "doc-agent" {
            assert_eq!(
                hit.trust_class,
                Some(TrustClass::AgentProposed),
                "doc-agent must carry AgentProposed trust_class"
            );
        }
        if hit.id == "doc-legacy" {
            // Legacy (missing stamp) → defaults to DocumentIngested.
            assert_eq!(
                hit.trust_class,
                Some(TrustClass::DocumentIngested),
                "legacy doc without stamp must default to DocumentIngested"
            );
        }
    }
}

#[tokio::test]
async fn document_only_filter_excludes_user_and_agent_hits() {
    // provenance_classes = [document_ingested] must exclude user_authored and
    // agent_proposed hits; legacy (no stamp) must still qualify.
    let (embedder, vectors) = trust_fixture().await;
    let retriever = trust_retriever(embedder, vectors);
    let query = KnowledgeQuery::try_new(
        "document",
        16,
        QueryFilter {
            provenance_classes: Some(vec![TrustClass::DocumentIngested]),
            ..QueryFilter::default()
        },
        None,
    )
    .expect("valid query");
    let hits = retriever.search(&query).await.expect("search");
    let ids: std::collections::BTreeSet<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert!(
        ids.contains("doc-ingested"),
        "ingested doc must pass: {ids:?}"
    );
    assert!(
        ids.contains("doc-legacy"),
        "legacy doc (no stamp → document_ingested) must pass: {ids:?}"
    );
    assert!(
        !ids.contains("doc-user"),
        "user_authored must be excluded: {ids:?}"
    );
    assert!(
        !ids.contains("doc-agent"),
        "agent_proposed must be excluded: {ids:?}"
    );
}

#[tokio::test]
async fn user_only_filter_returns_only_findings() {
    // provenance_classes = [user_authored] must return only the finding doc.
    let (embedder, vectors) = trust_fixture().await;
    let retriever = trust_retriever(embedder, vectors);
    let query = KnowledgeQuery::try_new(
        "document",
        16,
        QueryFilter {
            provenance_classes: Some(vec![TrustClass::UserAuthored]),
            ..QueryFilter::default()
        },
        None,
    )
    .expect("valid query");
    let hits = retriever.search(&query).await.expect("search");
    let ids: std::collections::BTreeSet<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert_eq!(
        ids,
        std::collections::BTreeSet::from(["doc-user"]),
        "only user_authored doc must appear"
    );
    assert_eq!(
        hits[0].trust_class,
        Some(TrustClass::UserAuthored),
        "hit must carry UserAuthored class for explainability"
    );
}

#[tokio::test]
async fn restrictive_filter_with_zero_matches_returns_empty_not_error() {
    // A filter that matches no indexed classes must yield an empty result, not an error.
    // Use rule_derived — no docs in the fixture carry that class.
    let (embedder, vectors) = trust_fixture().await;
    let retriever = trust_retriever(embedder, vectors);
    let query = KnowledgeQuery::try_new(
        "document",
        16,
        QueryFilter {
            provenance_classes: Some(vec![TrustClass::RuleDerived]),
            ..QueryFilter::default()
        },
        None,
    )
    .expect("valid query");
    let hits = retriever
        .search(&query)
        .await
        .expect("search must succeed, not error");
    assert!(
        hits.is_empty(),
        "0 hits at rule_derived trust level — not an error: {hits:?}"
    );
}

#[tokio::test]
async fn empty_provenance_classes_vec_is_same_as_none() {
    // An explicitly empty Vec is treated as "all classes" — same as None.
    let (embedder, vectors) = trust_fixture().await;
    let retriever = trust_retriever(embedder, vectors);
    let query = KnowledgeQuery::try_new(
        "document",
        16,
        QueryFilter {
            provenance_classes: Some(vec![]),
            ..QueryFilter::default()
        },
        None,
    )
    .expect("valid query");
    let hits = retriever.search(&query).await.expect("search");
    let ids: std::collections::BTreeSet<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    // All four docs must appear (empty set = all classes).
    assert!(ids.contains("doc-ingested"), "{ids:?}");
    assert!(ids.contains("doc-user"), "{ids:?}");
    assert!(ids.contains("doc-agent"), "{ids:?}");
    assert!(ids.contains("doc-legacy"), "{ids:?}");
}

#[tokio::test]
async fn trust_class_token_round_trip() {
    // All four TrustClass variants serialise to their payload tokens and
    // parse back exactly — no unknown tokens leak through.
    for (class, token) in [
        (TrustClass::DocumentIngested, "document_ingested"),
        (TrustClass::RuleDerived, "rule_derived"),
        (TrustClass::AgentProposed, "agent_proposed"),
        (TrustClass::UserAuthored, "user_authored"),
    ] {
        assert_eq!(class.payload_token(), token, "token mismatch for {class:?}");
        assert_eq!(
            TrustClass::from_payload_token(token),
            Some(class),
            "round-trip failed for {token}"
        );
    }
    assert_eq!(
        TrustClass::from_payload_token("unknown_garbage"),
        None,
        "unknown tokens must return None"
    );
}

/// Fixture for the graph-expansion trust-gate e2e test.
///
/// Layout:
/// - `doc-seed`   → `finding:seed`   — provenance_class: `user_authored`
/// - `doc-neighbor` → `instrument:neighbor` — NO provenance_class stamp
///   (backward-compat default → `DocumentIngested`)
/// - Graph edge: `finding:seed` -[related_to]→ `instrument:neighbor`
///
/// With a `user_authored`-only filter the seed passes, but the expanded
/// neighbour defaults to `DocumentIngested` and must be excluded by the
/// post-expansion retain pass (K-X3 fix #1).
async fn expand_trust_fixture() -> (
    Arc<HashEmbeddingProvider>,
    Arc<InMemoryVectorEngine>,
    Arc<InMemoryGraphEngine>,
) {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());

    // Seed: user_authored
    let seed_embedding = embedder
        .embed("seed finding doc")
        .await
        .expect("embed seed");
    vectors
        .upsert(
            COLLECTION,
            vec![VectorPoint {
                id: "doc-seed".to_string(),
                vector: seed_embedding.vector,
                payload: json!({
                    "entity_id": "finding:seed",
                    "tags": [],
                    "entity_kind": "finding",
                    "provenance_class": "user_authored",
                }),
            }],
        )
        .await
        .expect("upsert seed");

    // Neighbour: no provenance_class stamp → defaults to DocumentIngested
    let nbr_embedding = embedder
        .embed("neighbour instrument doc")
        .await
        .expect("embed neighbour");
    vectors
        .upsert(
            COLLECTION,
            vec![VectorPoint {
                id: "doc-neighbor".to_string(),
                vector: nbr_embedding.vector,
                payload: json!({
                    "entity_id": "instrument:neighbor",
                    "tags": [],
                    "entity_kind": "instrument",
                    // intentionally no provenance_class — legacy backward-compat path
                }),
            }],
        )
        .await
        .expect("upsert neighbour");

    // Graph: entity nodes + described_by edges + the expansion edge
    let seed_node = node("finding:seed", EntityKind::Finding);
    let nbr_node = node("instrument:neighbor", EntityKind::Instrument);
    let seed_doc_node = {
        let mut n = node("document:doc-seed", EntityKind::Document);
        n.props = json!({"plane": "platform"});
        n
    };
    let nbr_doc_node = {
        let mut n = node("document:doc-neighbor", EntityKind::Document);
        n.props = json!({"plane": "platform"});
        n
    };
    graph
        .upsert_nodes(vec![seed_node, nbr_node, seed_doc_node, nbr_doc_node])
        .await
        .expect("upsert nodes");
    graph
        .upsert_edges(vec![
            edge("finding:seed", "document:doc-seed", "described_by"),
            edge(
                "instrument:neighbor",
                "document:doc-neighbor",
                "described_by",
            ),
            edge("finding:seed", "instrument:neighbor", "related_to"),
        ])
        .await
        .expect("upsert edges");

    (embedder, vectors, graph)
}

#[tokio::test]
async fn trust_dial_expand_filter_gates_expanded_neighbors() {
    // K-X3 fix #1: graph-expanded neighbours must pass the trust-dial filter.
    // Setup: seed (user_authored) expands to neighbour (no stamp → document_ingested).
    // With user_authored filter: seed appears, neighbour must be excluded.
    // Without filter: neighbour appears (confirming expansion is active).
    let (embedder, vectors, graph) = expand_trust_fixture().await;

    let expansion = GraphExpansion {
        k_hop: 1,
        decay: 0.5,
        per_hit_limit: 10,
        edge_types: vec![],
    };

    // --- Without filter: neighbour IS reached via expansion ---
    let retriever_no_filter =
        Retriever::new(embedder.clone(), vectors.clone(), COLLECTION).with_graph(graph.clone());
    let query_no_filter = KnowledgeQuery::try_new(
        "seed finding doc",
        16,
        QueryFilter::default(),
        Some(expansion.clone()),
    )
    .expect("valid query");
    let hits_no_filter = retriever_no_filter
        .search(&query_no_filter)
        .await
        .expect("search without filter");
    let ids_no_filter: std::collections::BTreeSet<&str> =
        hits_no_filter.iter().map(|h| h.id.as_str()).collect();
    assert!(
        ids_no_filter.contains("doc-neighbor"),
        "without filter, expansion must reach doc-neighbor: {ids_no_filter:?}"
    );

    // --- With user_authored filter: neighbour must be excluded by retain pass ---
    let retriever_filtered =
        Retriever::new(embedder.clone(), vectors.clone(), COLLECTION).with_graph(graph.clone());
    let query_filtered = KnowledgeQuery::try_new(
        "seed finding doc",
        16,
        QueryFilter {
            provenance_classes: Some(vec![TrustClass::UserAuthored]),
            ..QueryFilter::default()
        },
        Some(expansion),
    )
    .expect("valid query");
    let hits_filtered = retriever_filtered
        .search(&query_filtered)
        .await
        .expect("search with user_authored filter");
    let ids_filtered: std::collections::BTreeSet<&str> =
        hits_filtered.iter().map(|h| h.id.as_str()).collect();
    assert!(
        ids_filtered.contains("doc-seed"),
        "seed (user_authored) must pass the filter: {ids_filtered:?}"
    );
    assert!(
        !ids_filtered.contains("doc-neighbor"),
        "neighbour (no stamp → document_ingested) must be excluded by \
         the post-expansion trust-dial retain pass: {ids_filtered:?}"
    );
}

/// Fixture for the tag-channel trust-gate e2e test.
///
/// Layout:
/// - `doc-finding` → `finding:tag-test` — vector payload has
///   `provenance_class: "user_authored"`; graph document node has
///   `provenance_class: "user_authored"` in props (K-X3 stamp).
/// - Tag `research:findings` assigned to `finding:tag-test`.
/// - The tag channel resolves `finding:tag-test` → `doc-finding` and must
///   carry the `user_authored` class from the graph node props.
///
/// With a `document_ingested`-only filter the finding doc must be EXCLUDED
/// even though it is the ONLY result reachable via the tag channel.
async fn tag_trust_fixture() -> (
    Arc<HashEmbeddingProvider>,
    Arc<InMemoryVectorEngine>,
    Arc<InMemoryGraphEngine>,
    Arc<InMemoryTagEngine>,
) {
    let embedder = Arc::new(HashEmbeddingProvider::default());
    let vectors = Arc::new(InMemoryVectorEngine::default());
    let graph = Arc::new(InMemoryGraphEngine::default());
    let tags = Arc::new(InMemoryTagEngine::default());

    // Vector point: user_authored finding
    let emb = embedder
        .embed("research finding note")
        .await
        .expect("embed");
    vectors
        .upsert(
            COLLECTION,
            vec![VectorPoint {
                id: "doc-finding".to_string(),
                vector: emb.vector,
                payload: json!({
                    "entity_id": "finding:tag-test",
                    "tags": ["research:findings"],
                    "entity_kind": "finding",
                    "provenance_class": "user_authored",
                    "as_of": "2026-06-01T00:00:00Z",
                    "plane": "platform",
                }),
            }],
        )
        .await
        .expect("upsert finding");

    // Graph: entity node + document node (with provenance_class in props)
    let entity_node = node("finding:tag-test", EntityKind::Finding);
    let mut doc_node = node("document:doc-finding", EntityKind::Document);
    // K-X3: provenance_class is stamped here by write_durable_graph; we mirror
    // that in the fixture so the tag channel can read it from node.props.
    doc_node.props = json!({
        "provenance_class": "user_authored",
        "as_of": "2026-06-01T00:00:00Z",
        "plane": "platform",
    });
    graph
        .upsert_nodes(vec![entity_node, doc_node])
        .await
        .expect("upsert nodes");
    graph
        .upsert_edges(vec![edge(
            "finding:tag-test",
            "document:doc-finding",
            "described_by",
        )])
        .await
        .expect("upsert edges");

    // Tags: define + assign research:findings to the finding entity
    tags.define(TagDefinition {
        tag_id: "research:findings".to_string(),
        parent: None,
        ttl_days: None,
    })
    .await
    .expect("define tag");
    tags.assign(TagAssignment {
        entity_id: "finding:tag-test".to_string(),
        tag_id: "research:findings".to_string(),
        assigned_at: "2026-06-01".to_string(),
        expires_at: None,
        provenance: "test:fixture".to_string(),
    })
    .await
    .expect("assign tag");

    (embedder, vectors, graph, tags)
}

#[tokio::test]
async fn trust_dial_tag_channel_gates_user_authored_under_document_only_filter() {
    // K-X3 Gemini review: tag-channel hits must carry their real provenance class
    // from graph node props, not always default to DocumentIngested.
    //
    // Setup: a Finding entity holds tag "research:findings".  Its document node
    // props carry provenance_class="user_authored" (stamped by write_durable_graph).
    // With a document_ingested-only filter the tag channel must exclude it.
    // Without the filter it must appear.
    let (embedder, vectors, graph, tags) = tag_trust_fixture().await;

    let retriever = Retriever::new(embedder.clone(), vectors.clone(), COLLECTION)
        .with_graph(graph.clone())
        .with_tags(tags.clone());

    // --- Without filter: finding appears via tag channel ---
    let query_no_filter = KnowledgeQuery::try_new(
        "research finding",
        16,
        QueryFilter {
            tags_any: vec!["research:findings".to_string()],
            as_of: Some("2026-06-12".to_string()),
            ..QueryFilter::default()
        },
        None,
    )
    .expect("valid query");
    let hits_no_filter = retriever
        .search(&query_no_filter)
        .await
        .expect("search without filter");
    let ids_no_filter: std::collections::BTreeSet<&str> =
        hits_no_filter.iter().map(|h| h.id.as_str()).collect();
    assert!(
        ids_no_filter.contains("doc-finding"),
        "without filter, tag channel must surface the finding doc: {ids_no_filter:?}"
    );

    // --- With document_ingested filter: finding excluded (it is user_authored) ---
    let query_filtered = KnowledgeQuery::try_new(
        "research finding",
        16,
        QueryFilter {
            tags_any: vec!["research:findings".to_string()],
            as_of: Some("2026-06-12".to_string()),
            provenance_classes: Some(vec![TrustClass::DocumentIngested]),
            ..QueryFilter::default()
        },
        None,
    )
    .expect("valid query");
    let hits_filtered = retriever
        .search(&query_filtered)
        .await
        .expect("search with document_ingested filter");
    let ids_filtered: std::collections::BTreeSet<&str> =
        hits_filtered.iter().map(|h| h.id.as_str()).collect();
    assert!(
        !ids_filtered.contains("doc-finding"),
        "document_ingested filter must exclude the user_authored finding \
         that arrives via the tag channel: {ids_filtered:?}"
    );
}
