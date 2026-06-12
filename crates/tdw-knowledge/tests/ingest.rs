//! End-to-end ingestion (knowledge-system B5): manifest idempotency,
//! rule-driven auto-tagging with graph context, lexical co-indexing, durable
//! graph stamping per the B4 visibility contract, and article conversion.

use std::sync::Arc;

use tdw_core::{Direction, GraphEngine, LexicalEngine, TextQuery, TraversalFilter};
use tdw_kg::{Entity, EntityKind};
use tdw_knowledge::indexer::{
    IndexManifest, IndexOutcome, KnowledgeIndexer, article_to_document, content_hash,
    date_from_epoch_ms,
};
use tdw_knowledge::{DocumentSource, KnowledgeDocument, KnowledgeIndex};
use tdw_news_compose::Article;
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_tag_rules::{Predicate, RuleEngine, TagRule};

const LEXICAL_INDEX: &str = "tdw_knowledge_text_test";

fn acme_doc() -> KnowledgeDocument {
    let mut document = KnowledgeDocument::new(
        "doc-acme-q1",
        "acme quarterly earnings beat expectations on cloud growth",
        Entity {
            entity_id: "instrument:ACME".to_string(),
            kind: EntityKind::Instrument,
            label: "Acme Corp".to_string(),
            aliases: vec!["ACME".to_string()],
        },
        vec!["asset:equity".to_string()],
    );
    document.plane = Some("platform".to_string());
    document.as_of = Some("2026-03-05".to_string());
    document.mentions = vec!["instrument:BETA".to_string()];
    document.source = Some(DocumentSource::Provider {
        provider: "sec".to_string(),
    });
    document
}

fn indexer_with_graph() -> (KnowledgeIndexer, Arc<InMemoryGraphEngine>) {
    let graph = Arc::new(InMemoryGraphEngine::default());
    let indexer = KnowledgeIndexer::new(KnowledgeIndex::default()).with_graph(graph.clone());
    (indexer, graph)
}

#[tokio::test]
async fn reindex_is_idempotent_until_content_changes() {
    let (mut indexer, _graph) = indexer_with_graph();
    let document = acme_doc();
    assert_eq!(
        indexer
            .index_at(document.clone(), "2026-03-06")
            .await
            .expect("index"),
        IndexOutcome::Indexed
    );
    assert_eq!(
        indexer
            .index_at(document.clone(), "2026-03-07")
            .await
            .expect("re-index"),
        IndexOutcome::SkippedUnchanged,
        "unchanged content must not re-write anything"
    );
    // A content change re-indexes.
    let mut changed = document;
    changed.body.push_str(" — updated guidance");
    assert_eq!(
        indexer
            .index_at(changed, "2026-03-07")
            .await
            .expect("changed"),
        IndexOutcome::Indexed
    );
    // Re-indexing did NOT duplicate the tag assignment (append-only store).
    let active = indexer.index().active_tags("instrument:ACME", "2026-03-07");
    assert_eq!(active, vec!["asset:equity".to_string()]);
}

#[tokio::test]
async fn manifest_round_trips_through_json() {
    let (mut indexer, _graph) = indexer_with_graph();
    indexer
        .index_at(acme_doc(), "2026-03-06")
        .await
        .expect("index");
    let raw = indexer.manifest().to_json().expect("serialize");
    let restored = IndexManifest::from_json(&raw).expect("deserialize");
    assert_eq!(&restored, indexer.manifest());
    assert!(restored.is_current("doc-acme-q1", &content_hash(&acme_doc())));
}

#[tokio::test]
async fn rules_auto_tag_and_payload_sees_the_tags() {
    let mut rules = RuleEngine::default();
    rules
        .hot_reload(vec![TagRule {
            rule_id: "earnings-mention".to_string(),
            tag_id: "topic:earnings".to_string(),
            predicate: Predicate::FieldContains {
                field: "body".to_string(),
                needle: "earnings".to_string(),
            },
        }])
        .expect("valid rule");
    let (indexer, _graph) = indexer_with_graph();
    let mut indexer = indexer.with_rules(rules).expect("with rules");
    // Rule tags must be DEFINED before any rule fires (rules assign, they
    // never define); a setup document carrying the tag defines it through
    // the public index path.
    let mut seed = acme_doc();
    seed.id = "doc-taxonomy-seed".to_string();
    seed.tags = vec!["topic:earnings".to_string()];
    seed.body = "taxonomy seed".to_string();
    indexer
        .index_at(seed, "2026-03-01")
        .await
        .expect("seed defines topic:earnings");

    let outcome = indexer
        .index_at(acme_doc(), "2026-03-06")
        .await
        .expect("index");
    assert_eq!(outcome, IndexOutcome::Indexed);
    let active = indexer.index().active_tags("instrument:ACME", "2026-03-06");
    assert!(
        active.contains(&"topic:earnings".to_string()),
        "rule must have fired: {active:?}"
    );
    let hits = indexer
        .index()
        .search(
            "acme quarterly earnings beat expectations on cloud growth",
            2,
        )
        .await
        .expect("search");
    let hit = hits
        .iter()
        .find(|hit| hit.id == "doc-acme-q1")
        .expect("indexed doc is searchable");
    assert!(
        hit.tags.contains(&"topic:earnings".to_string()),
        "payload tags must include rule-assigned tags: {:?}",
        hit.tags
    );
}

#[tokio::test]
async fn lexical_co_index_carries_the_payload_contract() {
    let lexical = Arc::new(InMemoryLexicalEngine::default());
    let mut indexer = KnowledgeIndexer::new(KnowledgeIndex::default())
        .with_lexical(lexical.clone(), LEXICAL_INDEX);
    indexer
        .index_at(acme_doc(), "2026-03-06")
        .await
        .expect("index");
    let docs = lexical
        .search_text(LEXICAL_INDEX, TextQuery::text("acme", 5))
        .await
        .expect("lexical search");
    assert_eq!(docs.len(), 1);
    let fields = &docs[0].fields;
    assert_eq!(fields["entity_id"], "instrument:ACME");
    assert_eq!(fields["entity_kind"], "instrument");
    assert_eq!(fields["plane"], "platform");
    assert_eq!(
        fields["as_of"], "2026-03-05T00:00:00Z",
        "payload as_of is the normalized timestamp"
    );
    assert!(fields["content_hash"].is_string());
}

#[tokio::test]
async fn durable_graph_carries_visibility_props_and_mention_stubs() {
    let (mut indexer, graph) = indexer_with_graph();
    indexer
        .index_at(acme_doc(), "2026-03-06")
        .await
        .expect("index");

    let document_node = graph
        .node("document:doc-acme-q1")
        .await
        .expect("graph read")
        .expect("document node exists");
    assert_eq!(document_node.kind, EntityKind::Document);
    assert_eq!(document_node.props["plane"], "platform");
    assert_eq!(
        document_node.props["as_of"], "2026-03-05T00:00:00Z",
        "B4 document_visible reads props.as_of — this is the leakage gate"
    );

    // An UNDATED document gets NO as_of prop (stays temporally invisible).
    let mut undated = acme_doc();
    undated.id = "doc-acme-undated".to_string();
    undated.as_of = None;
    indexer
        .index_at(undated, "2026-03-06")
        .await
        .expect("index undated");
    let undated_node = graph
        .node("document:doc-acme-undated")
        .await
        .expect("graph read")
        .expect("undated document node");
    assert!(undated_node.props.get("as_of").is_none());

    // Mention stub: created as Primitive, marked, and never clobbering.
    let stub = graph
        .node("instrument:BETA")
        .await
        .expect("graph read")
        .expect("mention stub exists");
    assert_eq!(stub.kind, EntityKind::Primitive);
    assert_eq!(stub.props["mention_stub"], true);

    let neighbors = graph
        .neighbors(
            "instrument:ACME",
            &TraversalFilter {
                rels: Some(vec!["mentions".to_string()]),
                direction: Direction::Out,
                max_hops: 1,
                ..TraversalFilter::default()
            },
        )
        .await
        .expect("neighbors");
    // Both documents wrote a mentions edge (distinct validity identities —
    // the dated one carries valid_from, the undated one is open).
    assert_eq!(neighbors.len(), 2);
    assert!(
        neighbors
            .iter()
            .all(|(_, node)| node.id == "instrument:BETA"),
        "every mentions edge reaches the stub"
    );
}

#[tokio::test]
async fn mention_stub_never_overwrites_a_real_node() {
    let (mut indexer, graph) = indexer_with_graph();
    graph
        .upsert_nodes(vec![tdw_core::GraphNode {
            id: "instrument:BETA".to_string(),
            kind: EntityKind::Instrument,
            label: "Beta Industries".to_string(),
            aliases: vec!["BETA".to_string()],
            props: serde_json::Value::Null,
            valid_from: None,
            valid_to: None,
        }])
        .await
        .expect("seed real node");
    indexer
        .index_at(acme_doc(), "2026-03-06")
        .await
        .expect("index");
    let node = graph
        .node("instrument:BETA")
        .await
        .expect("graph read")
        .expect("node");
    assert_eq!(node.kind, EntityKind::Instrument, "real node untouched");
    assert_eq!(node.label, "Beta Industries");
}

#[test]
fn article_converts_with_stable_id_date_and_mentions() {
    let article = Article::new(
        "Acme beats on earnings",
        "https://example.com/acme-q1",
        "Benzinga",
        1_773_100_800_000, // 2026-03-10T00:00:00Z
        "Cloud growth drove the quarter.",
        vec!["ACME".to_string(), "BETA".to_string()],
    );
    let document = article_to_document(&article, "platform");
    let again = article_to_document(&article, "platform");
    assert_eq!(document.id, again.id, "url-hashed id is stable");
    assert!(document.id.starts_with("news-"));
    assert_eq!(document.entity.kind, EntityKind::Document);
    assert_eq!(document.plane.as_deref(), Some("platform"));
    assert_eq!(document.as_of.as_deref(), Some("2026-03-10"));
    assert_eq!(
        document.mentions,
        vec!["instrument:ACME".to_string(), "instrument:BETA".to_string()]
    );
    assert!(document.body.contains("Acme beats on earnings"));
    assert!(matches!(document.source, Some(DocumentSource::News { .. })));
}

#[tokio::test]
async fn rule_tags_do_not_duplicate_across_reindexes() {
    let mut rules = RuleEngine::default();
    rules
        .hot_reload(vec![TagRule {
            rule_id: "earnings-mention".to_string(),
            tag_id: "topic:earnings".to_string(),
            predicate: Predicate::FieldContains {
                field: "body".to_string(),
                needle: "earnings".to_string(),
            },
        }])
        .expect("valid rule");
    let (indexer, _graph) = indexer_with_graph();
    let mut indexer = indexer.with_rules(rules).expect("with rules");
    let mut seed = acme_doc();
    seed.id = "doc-taxonomy-seed".to_string();
    seed.tags = vec!["topic:earnings".to_string()];
    seed.body = "taxonomy seed".to_string();
    indexer.index_at(seed, "2026-03-01").await.expect("seed");

    // Three content-changing re-indexes: the rule fires each time, but the
    // already-active guard must keep exactly ONE assignment.
    for round in 0..3 {
        let mut document = acme_doc();
        document.body = format!("acme earnings update round {round}");
        indexer
            .index_at(document, "2026-03-06")
            .await
            .expect("re-index");
    }
    let active = indexer.index().active_tags("instrument:ACME", "2026-03-06");
    let earnings = active
        .iter()
        .filter(|tag| tag.as_str() == "topic:earnings")
        .count();
    assert_eq!(earnings, 1, "rule tag must not duplicate: {active:?}");
}

#[test]
fn content_hash_tracks_entity_identity() {
    let document = acme_doc();
    let mut moved = acme_doc();
    moved.entity.entity_id = "instrument:ACME-NEW".to_string();
    assert_ne!(
        content_hash(&document),
        content_hash(&moved),
        "entity re-resolution must invalidate the manifest entry"
    );
}

#[tokio::test]
async fn self_mention_neither_clobbers_entity_nor_writes_a_self_loop() {
    let (mut indexer, graph) = indexer_with_graph();
    let mut document = acme_doc();
    document.mentions = vec![
        "instrument:ACME".to_string(), // self
        "instrument:BETA".to_string(),
    ];
    indexer
        .index_at(document, "2026-03-06")
        .await
        .expect("index");
    let entity = graph
        .node("instrument:ACME")
        .await
        .expect("graph read")
        .expect("entity node");
    assert_eq!(
        entity.kind,
        EntityKind::Instrument,
        "no Primitive stub clobber"
    );
    assert!(entity.props.get("mention_stub").is_none());
    let mentions = graph
        .neighbors(
            "instrument:ACME",
            &TraversalFilter {
                rels: Some(vec!["mentions".to_string()]),
                direction: Direction::Out,
                max_hops: 1,
                ..TraversalFilter::default()
            },
        )
        .await
        .expect("neighbors");
    assert!(
        mentions
            .iter()
            .all(|(_, node)| node.id == "instrument:BETA"),
        "no self-loop mentions edge"
    );
}

#[test]
fn hostile_article_title_is_sanitized() {
    let article = Article::new(
        "Acme beats\u{1b}[2J\u{7}\nFAKE: SEC HALTS TRADING",
        "https://example.com/hostile",
        "wire",
        1_773_100_800_000,
        "summary\u{0}with nul",
        vec!["ACME".to_string()],
    );
    let document = article_to_document(&article, "platform");
    assert!(
        !document.entity.label.chars().any(char::is_control),
        "control characters must be stripped at the trust boundary"
    );
    assert!(document.entity.label.contains("Acme beats"));
    assert!(!document.body.contains('\u{0}'));
    // And the validators reject any label that slips through another path.
    let mut forged = acme_doc();
    forged.entity.label = "evil\u{1b}[2J".to_string();
    assert!(tdw_knowledge::validate_document(&forged).is_err());
}

#[test]
fn epoch_to_date_handles_boundaries() {
    assert_eq!(date_from_epoch_ms(0), "1970-01-01");
    assert_eq!(date_from_epoch_ms(86_400_000 - 1), "1970-01-01");
    assert_eq!(date_from_epoch_ms(86_400_000), "1970-01-02");
    assert_eq!(
        date_from_epoch_ms(-1),
        "1969-12-31",
        "pre-epoch floors earlier"
    );
    // Leap day.
    assert_eq!(date_from_epoch_ms(1_709_164_800_000), "2024-02-29");
}
