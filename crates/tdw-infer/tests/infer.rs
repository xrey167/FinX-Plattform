//! End-to-end inference tests over the in-memory graph + tag reference engines.

use std::collections::BTreeSet;
use std::sync::Arc;

use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance, TraversalFilter};
use tdw_infer::{
    ChangeSet, Derivation, DerivationIndex, EdgePattern, InferEngine, InferError, InferRule,
    RunLimits, edge_key, tag_key,
};
use tdw_storage_graph::InMemoryGraphEngine;
use tdw_tags::{InMemoryTagEngine, TagAssignment, TagDefinition, TagEngine};
use tdw_taxonomy::EntityKind;

const NOW: &str = "2026-06-10";

fn node(id: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind: EntityKind::Instrument,
        label: id.to_string(),
        aliases: Vec::new(),
        props: serde_json::Value::Null,
        valid_from: None,
        valid_to: None,
    }
}

fn edge(from: &str, rel: &str, to: &str) -> GraphEdge {
    GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
        rel: rel.to_string(),
        props: serde_json::Value::Null,
        provenance: Provenance::Ingest {
            source: "fixture".to_string(),
        },
        valid_from: None,
        valid_to: None,
    }
}

async fn seed_graph(nodes: &[&str], edges: Vec<GraphEdge>) -> Arc<dyn GraphEngine> {
    let graph = InMemoryGraphEngine::default();
    graph
        .upsert_nodes(nodes.iter().map(|id| node(id)).collect())
        .await
        .expect("nodes");
    if !edges.is_empty() {
        graph.upsert_edges(edges).await.expect("edges");
    }
    Arc::new(graph)
}

fn pattern(rels: &[&str]) -> Vec<EdgePattern> {
    rels.iter()
        .map(|rel| EdgePattern {
            rel: (*rel).to_string(),
        })
        .collect()
}

async fn rel_count(graph: &Arc<dyn GraphEngine>, rel: &str) -> usize {
    graph.edges(Some(rel), 0, 1024).await.expect("scan").len()
}

#[tokio::test]
async fn chain_derivation_is_idempotent_and_skips_self_loops() {
    // A -supplier_of-> B -listed_on-> X  =>  A -exposed_to-> X
    // Plus a self-loop fixture: S -supplier_of-> S -listed_on-> S must NOT derive S->S.
    let graph = seed_graph(
        &["a", "b", "x", "s"],
        vec![
            edge("a", "supplier_of", "b"),
            edge("b", "listed_on", "x"),
            edge("s", "supplier_of", "s"),
            edge("s", "listed_on", "s"),
        ],
    )
    .await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());

    let mut engine = InferEngine::default();
    engine
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "exposure".to_string(),
            stratum: 0,
            when: pattern(&["supplier_of", "listed_on"]),
            derived_type: "exposed_to".to_string(),
        }])
        .expect("reload");

    let report = engine.run_full(&graph, &tags, NOW).await.expect("run");
    assert_eq!(report.derived_edges, 1, "one chain, self-loop skipped");
    assert_eq!(report.version, 1);

    // The derived edge exists with Rule provenance.
    let derived = graph
        .neighbors(
            "a",
            &TraversalFilter {
                rels: Some(vec!["exposed_to".to_string()]),
                ..TraversalFilter::default()
            },
        )
        .await
        .expect("neighbors");
    assert_eq!(derived.len(), 1);
    assert_eq!(derived[0].1.id, "x");
    assert_eq!(
        derived[0].0.provenance,
        Provenance::Rule {
            rule_id: "exposure".to_string(),
            version: 1,
        }
    );
    // No self-loop edge was derived for s.
    let self_derived = graph
        .edges(Some("exposed_to"), 0, 64)
        .await
        .expect("scan")
        .into_iter()
        .filter(|e| e.from == "s")
        .count();
    assert_eq!(self_derived, 0);

    // Re-run derives nothing new (idempotent dedup via the derivation index).
    let again = engine.run_full(&graph, &tags, NOW).await.expect("rerun");
    assert_eq!(again.derived_edges, 0);
    assert_eq!(rel_count(&graph, "exposed_to").await, 1);
}

#[tokio::test]
async fn stratified_cascade_derives_two_hops_and_rejects_bad_rule_sets() {
    // stratum 0: a -r1-> b -r2-> c  => a -mid-> c
    // stratum 1: a -mid-> c -r3-> d => a -top-> d   (consumes stratum-0 derived "mid")
    let graph = seed_graph(
        &["a", "b", "c", "d"],
        vec![
            edge("a", "r1", "b"),
            edge("b", "r2", "c"),
            edge("c", "r3", "d"),
        ],
    )
    .await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());

    let mut engine = InferEngine::default();
    engine
        .hot_reload(vec![
            InferRule::DeriveEdge {
                rule_id: "lower".to_string(),
                stratum: 0,
                when: pattern(&["r1", "r2"]),
                derived_type: "mid".to_string(),
            },
            InferRule::DeriveEdge {
                rule_id: "upper".to_string(),
                stratum: 1,
                when: pattern(&["mid", "r3"]),
                derived_type: "top".to_string(),
            },
        ])
        .expect("stratified reload");

    let report = engine.run_full(&graph, &tags, NOW).await.expect("run");
    assert_eq!(report.derived_edges, 2, "mid then top");
    assert_eq!(rel_count(&graph, "mid").await, 1);
    assert_eq!(rel_count(&graph, "top").await, 1);
    let top = graph.edges(Some("top"), 0, 8).await.expect("scan");
    assert_eq!((top[0].from.as_str(), top[0].to.as_str()), ("a", "d"));

    // Same-stratum consumption of a derived type is rejected at hot_reload.
    let mut bad = InferEngine::default();
    let same_stratum = bad.hot_reload(vec![
        InferRule::DeriveEdge {
            rule_id: "lower".to_string(),
            stratum: 0,
            when: pattern(&["r1", "r2"]),
            derived_type: "mid".to_string(),
        },
        InferRule::DeriveEdge {
            rule_id: "upper".to_string(),
            stratum: 0,
            when: pattern(&["mid", "r3"]),
            derived_type: "top".to_string(),
        },
    ]);
    assert!(matches!(
        same_stratum,
        Err(InferError::Unstratifiable { .. })
    ));

    // A self-recursive rule (derives a type it consumes) is rejected.
    let self_rec = bad.hot_reload(vec![InferRule::DeriveEdge {
        rule_id: "loop".to_string(),
        stratum: 0,
        when: pattern(&["x", "loops_to"]),
        derived_type: "loops_to".to_string(),
    }]);
    assert!(matches!(self_rec, Err(InferError::InvalidRule { .. })));
}

#[tokio::test]
async fn propagate_tag_subsumes_descendants_skips_active_and_stamps_provenance() {
    // owner --owned_by--> sub1 --owned_by--> sub2
    let graph = seed_graph(
        &["owner", "sub1", "sub2", "other"],
        vec![
            edge("owner", "owned_by", "sub1"),
            edge("sub1", "owned_by", "sub2"),
        ],
    )
    .await;

    let tag_engine = InMemoryTagEngine::default();
    // Taxonomy: risk:any -> risk:sanctioned. Seed by the DESCENDANT to prove subsumption.
    for (tag, parent) in [("risk:any", None), ("risk:sanctioned", Some("risk:any"))] {
        tag_engine
            .define(TagDefinition {
                tag_id: tag.to_string(),
                parent: parent.map(ToString::to_string),
                ttl_days: None,
            })
            .await
            .expect("define");
    }
    tag_engine
        .assign(TagAssignment {
            entity_id: "owner".to_string(),
            tag_id: "risk:sanctioned".to_string(),
            assigned_at: "2026-01-01".to_string(),
            expires_at: None,
            provenance: "fixture".to_string(),
        })
        .await
        .expect("assign");
    let tags: Arc<dyn TagEngine> = Arc::new(tag_engine);

    let mut engine = InferEngine::default();
    // Propagate the BASE tag risk:any (include_descendants picks up risk:sanctioned
    // holders too) along owned_by up to 2 hops.
    engine
        .hot_reload(vec![InferRule::PropagateTag {
            rule_id: "sanction-flow".to_string(),
            stratum: 0,
            tag: "risk:any".to_string(),
            include_descendants: true,
            along: vec!["owned_by".to_string()],
            outbound: true,
            max_hops: 2,
        }])
        .expect("reload");
    // risk:any must be defined for the assign to succeed (assign requires a known tag).
    let report = engine.run_full(&graph, &tags, NOW).await.expect("run");
    assert_eq!(report.assigned_tags, 2, "sub1 and sub2 within 2 hops");

    for entity in ["sub1", "sub2"] {
        let active = tags.active_tags(entity, NOW).await.expect("active");
        assert!(
            active.contains(&"risk:any".to_string()),
            "{entity}: {active:?}"
        );
    }
    // other is unreachable.
    assert!(
        tags.active_tags("other", NOW)
            .await
            .expect("active")
            .is_empty()
    );

    // Re-run: already-active skip means no duplicate assignments.
    let again = engine.run_full(&graph, &tags, NOW).await.expect("rerun");
    assert_eq!(again.assigned_tags, 0);

    // Provenance is derived:rule:<id>@v<version>.
    let derivation = engine
        .derivation_index()
        .get(&tag_key("sub1", "risk:any"))
        .expect("recorded");
    assert_eq!(derivation.rule_id, "sanction-flow");
    assert_eq!(derivation.version, 1);
}

#[tokio::test]
async fn limits_are_errors_not_silent_truncation() {
    // Fan-out graph: hub -fan-> {l0..l9}, each li -spoke-> sink. Chain fan+spoke derives
    // hub -reaches-> sink (one match per leaf, dedups to one edge), so to trip max_derived
    // we instead build many distinct sinks.
    let mut nodes = vec!["hub".to_string()];
    let mut edges = Vec::new();
    for i in 0..20 {
        let leaf = format!("leaf{i}");
        let sink = format!("sink{i}");
        nodes.push(leaf.clone());
        nodes.push(sink.clone());
        edges.push(edge("hub", "fan", &leaf));
        edges.push(edge(&leaf, "spoke", &sink));
    }
    let node_refs: Vec<&str> = nodes.iter().map(String::as_str).collect();
    let graph = seed_graph(&node_refs, edges).await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());

    // max_derived tiny: 20 distinct hub->sink edges exceed a cap of 5.
    let mut engine = InferEngine::with_limits(RunLimits {
        max_iterations: 32,
        max_derived: 5,
    });
    engine
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "reach".to_string(),
            stratum: 0,
            when: pattern(&["fan", "spoke"]),
            derived_type: "reaches".to_string(),
        }])
        .expect("reload");
    assert!(matches!(
        engine.run_full(&graph, &tags, NOW).await,
        Err(InferError::DerivedLimitExceeded { .. })
    ));

    // max_iterations tiny: zero allowed iterations trips immediately.
    let mut tight = InferEngine::with_limits(RunLimits {
        max_iterations: 0,
        max_derived: 10_000,
    });
    tight
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "reach".to_string(),
            stratum: 0,
            when: pattern(&["fan", "spoke"]),
            derived_type: "reaches".to_string(),
        }])
        .expect("reload");
    assert!(matches!(
        tight.run_full(&graph, &tags, NOW).await,
        Err(InferError::IterationLimitExceeded { .. })
    ));
}

#[tokio::test]
async fn run_incremental_fires_only_rules_touching_the_change_set() {
    let graph = seed_graph(
        &["a", "b", "x"],
        vec![edge("a", "supplier_of", "b"), edge("b", "listed_on", "x")],
    )
    .await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());

    let mut engine = InferEngine::default();
    engine
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "exposure".to_string(),
            stratum: 0,
            when: pattern(&["supplier_of", "listed_on"]),
            derived_type: "exposed_to".to_string(),
        }])
        .expect("reload");

    // A change set NOT mentioning the rule's inputs does not fire it.
    let unrelated = ChangeSet {
        edge_types: std::iter::once("unrelated_rel".to_string()).collect(),
        tags: BTreeSet::default(),
    };
    let no_fire = engine
        .run_incremental(&graph, &tags, NOW, &unrelated)
        .await
        .expect("run");
    assert_eq!(no_fire.derived_edges, 0);
    assert_eq!(rel_count(&graph, "exposed_to").await, 0);

    // One that DOES mention an input fires it.
    let related = ChangeSet {
        edge_types: std::iter::once("supplier_of".to_string()).collect(),
        tags: BTreeSet::default(),
    };
    let fired = engine
        .run_incremental(&graph, &tags, NOW, &related)
        .await
        .expect("run");
    assert_eq!(fired.derived_edges, 1);
    assert_eq!(rel_count(&graph, "exposed_to").await, 1);
}

#[tokio::test]
async fn retract_deletes_edges_transitively_and_reports_unremovable_tags() {
    // stratum-0 mid from r1,r2; stratum-1 top from mid,r3. Retracting the base r1 fact
    // must cascade: mid (support includes r1) and top (support includes mid) both go.
    let graph = seed_graph(
        &["a", "b", "c", "d"],
        vec![
            edge("a", "r1", "b"),
            edge("b", "r2", "c"),
            edge("c", "r3", "d"),
        ],
    )
    .await;
    let tag_engine = InMemoryTagEngine::default();
    tag_engine
        .define(TagDefinition {
            tag_id: "risk:x".to_string(),
            parent: None,
            ttl_days: None,
        })
        .await
        .expect("define");
    tag_engine
        .assign(TagAssignment {
            entity_id: "a".to_string(),
            tag_id: "risk:x".to_string(),
            assigned_at: "2026-01-01".to_string(),
            expires_at: None,
            provenance: "fixture".to_string(),
        })
        .await
        .expect("assign");
    let tags: Arc<dyn TagEngine> = Arc::new(tag_engine);

    let mut engine = InferEngine::default();
    engine
        .hot_reload(vec![
            InferRule::DeriveEdge {
                rule_id: "lower".to_string(),
                stratum: 0,
                when: pattern(&["r1", "r2"]),
                derived_type: "mid".to_string(),
            },
            InferRule::DeriveEdge {
                rule_id: "upper".to_string(),
                stratum: 1,
                when: pattern(&["mid", "r3"]),
                derived_type: "top".to_string(),
            },
            InferRule::PropagateTag {
                rule_id: "flow".to_string(),
                stratum: 2,
                tag: "risk:x".to_string(),
                include_descendants: false,
                along: vec!["r1".to_string()],
                outbound: true,
                max_hops: 1,
            },
        ])
        .expect("reload");
    let report = engine.run_full(&graph, &tags, NOW).await.expect("run");
    assert_eq!(report.derived_edges, 2);
    assert_eq!(report.assigned_tags, 1, "risk:x flows a -r1-> b");
    assert_eq!(rel_count(&graph, "mid").await, 1);
    assert_eq!(rel_count(&graph, "top").await, 1);

    // Retract the base edge fact a -r1-> b. Both derived edges depend on it transitively.
    let retract = engine
        .retract(&graph, &tags, &edge_key("a", "r1", "b"))
        .await
        .expect("retract");
    assert_eq!(retract.removed_edges, 2, "mid + top deleted");
    assert_eq!(rel_count(&graph, "mid").await, 0);
    assert_eq!(rel_count(&graph, "top").await, 0);

    // The propagated tag (support = tag:b:risk:x... actually seed is a) is retracted by the
    // base TAG fact, not the edge. Retracting the tag's base shows the unremovable report.
    let tag_retract = engine
        .retract(&graph, &tags, &tag_key("a", "risk:x"))
        .await
        .expect("retract tag");
    assert_eq!(tag_retract.removed_edges, 0);
    assert_eq!(tag_retract.unremovable_tags, vec![tag_key("b", "risk:x")]);
}

#[test]
fn derivation_index_round_trips() {
    let mut index = DerivationIndex::default();
    index.record(
        edge_key("a", "exposed_to", "x"),
        Derivation {
            rule_id: "exposure".to_string(),
            version: 3,
            support: vec![
                edge_key("a", "supplier_of", "b"),
                edge_key("b", "listed_on", "x"),
            ],
        },
    );
    let json = index.to_json().expect("to_json");
    let restored = DerivationIndex::from_json(&json).expect("from_json");
    assert_eq!(restored, index);
}

/// 3-lens review B1 (precursor): deriving over an existing same-triple edge
/// must neither clobber its provenance nor enter the derivation index.
#[tokio::test]
async fn derivation_never_reasserts_an_existing_triple() {
    let graph = seed_graph(
        &["a", "b", "x"],
        vec![
            edge("a", "supplier_of", "b"),
            edge("b", "listed_on", "x"),
            // Pre-existing BASE fact with the exact shape the rule derives.
            edge("a", "exposed_to", "x"),
        ],
    )
    .await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());
    let mut engine = InferEngine::default();
    engine
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "exposure".to_string(),
            stratum: 0,
            when: pattern(&["supplier_of", "listed_on"]),
            derived_type: "exposed_to".to_string(),
        }])
        .expect("rules");
    let report = engine.run_full(&graph, &tags, NOW).await.expect("run");
    assert_eq!(
        report.derived_edges, 0,
        "existing triple is never re-asserted"
    );
    let exposed = graph.edges(Some("exposed_to"), 0, 16).await.expect("scan");
    assert_eq!(exposed.len(), 1);
    assert!(
        matches!(exposed[0].provenance, Provenance::Ingest { .. }),
        "base provenance untouched: {:?}",
        exposed[0].provenance
    );
    assert!(
        !engine
            .derivation_index()
            .contains(&edge_key("a", "exposed_to", "x")),
        "a base fact never enters the derivation index"
    );
}

/// 3-lens review B1: retraction removes ONLY Rule-provenance edges; a base
/// edge sharing the (from, rel, to) shape — even with a different
/// `valid_from` — survives.
#[tokio::test]
async fn retract_preserves_same_shaped_base_edges() {
    let graph = seed_graph(
        &["a", "b", "x"],
        vec![edge("a", "supplier_of", "b"), edge("b", "listed_on", "x")],
    )
    .await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());
    let mut engine = InferEngine::default();
    engine
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "exposure".to_string(),
            stratum: 0,
            when: pattern(&["supplier_of", "listed_on"]),
            derived_type: "exposed_to".to_string(),
        }])
        .expect("rules");
    let report = engine.run_full(&graph, &tags, NOW).await.expect("run");
    assert_eq!(report.derived_edges, 1);

    // A human-authored edge with the same shape lands AFTER derivation,
    // under a different validity identity.
    let mut base = edge("a", "exposed_to", "x");
    base.valid_from = Some("2020-01-01T00:00:00Z".to_string());
    graph.upsert_edges(vec![base]).await.expect("base edge");
    assert_eq!(rel_count(&graph, "exposed_to").await, 2);

    let retract = engine
        .retract(&graph, &tags, &edge_key("a", "supplier_of", "b"))
        .await
        .expect("retract");
    assert_eq!(retract.removed_edges, 1, "only the derived edge is removed");
    let survivors = graph.edges(Some("exposed_to"), 0, 16).await.expect("scan");
    assert_eq!(survivors.len(), 1);
    assert!(
        matches!(survivors[0].provenance, Provenance::Ingest { .. }),
        "the base fact survives retraction: {:?}",
        survivors[0].provenance
    );
}

/// A restored (possibly forged/stale) derivation index cannot make `retract`
/// delete a base edge: the Rule-provenance check preserves it and reports it.
#[tokio::test]
async fn forged_index_cannot_delete_base_edges() {
    let graph = seed_graph(
        &["a", "b", "x"],
        vec![
            edge("a", "supplier_of", "b"),
            edge("a", "exposed_to", "x"), // base fact, Ingest provenance
        ],
    )
    .await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());
    let mut forged = DerivationIndex::default();
    forged.record(
        edge_key("a", "exposed_to", "x"),
        Derivation {
            rule_id: "evil".to_string(),
            version: 1,
            support: vec![edge_key("a", "supplier_of", "b")],
        },
    );
    let mut engine = InferEngine::default().with_derivation_index(forged);
    let report = engine
        .retract(&graph, &tags, &edge_key("a", "supplier_of", "b"))
        .await
        .expect("retract");
    assert_eq!(report.removed_edges, 0);
    assert_eq!(
        report.preserved_base_edges,
        vec![edge_key("a", "exposed_to", "x")]
    );
    assert_eq!(rel_count(&graph, "exposed_to").await, 1, "base edge intact");
}

/// K-X6 F4 regression: a user-authored edge (`Provenance::Agent { gated: false }`)
/// that matches a `DeriveEdge` pattern must NOT produce a derived fact when
/// `exclude_user_authored = true` (the default). Enabling opt-in via
/// `with_user_authored_inference(true)` DOES allow derivation.
#[tokio::test]
async fn user_authored_edges_excluded_from_derive_edge_by_default() {
    // user -researched-> company -listed_on-> exchange
    // Rule: researched + listed_on => watches
    // The "researched" edge is user-authored (Agent provenance, gated=false) —
    // represents a finding edge. By default it must NOT drive derivation.
    let graph = seed_graph(
        &["user1", "company1", "exchange1"],
        vec![
            // user-authored finding edge
            {
                let mut e = edge("user1", "researched", "company1");
                e.provenance = Provenance::Agent {
                    agent_id: "user:analyst".to_string(),
                    gated: false,
                };
                e
            },
            // ordinary ingest edge
            edge("company1", "listed_on", "exchange1"),
        ],
    )
    .await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());

    // Default engine: exclude_user_authored = true → no derivation.
    let mut default_engine = InferEngine::default();
    default_engine
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "watches".to_string(),
            stratum: 0,
            when: pattern(&["researched", "listed_on"]),
            derived_type: "watches".to_string(),
        }])
        .expect("reload");
    let report = default_engine
        .run_full(&graph, &tags, NOW)
        .await
        .expect("run");
    assert_eq!(
        report.derived_edges, 0,
        "user-authored edge excluded: no derivation by default"
    );
    assert_eq!(rel_count(&graph, "watches").await, 0);

    // Opt-in engine: with_user_authored_inference(true) → derivation fires.
    let mut opt_in_engine = InferEngine::default().with_user_authored_inference(true);
    opt_in_engine
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "watches".to_string(),
            stratum: 0,
            when: pattern(&["researched", "listed_on"]),
            derived_type: "watches".to_string(),
        }])
        .expect("reload");
    let opt_in_report = opt_in_engine
        .run_full(&graph, &tags, NOW)
        .await
        .expect("opt-in run");
    assert_eq!(
        opt_in_report.derived_edges, 1,
        "opt-in: user-authored edge participates in derivation"
    );
    assert_eq!(rel_count(&graph, "watches").await, 1);
}

/// 3-lens review N1: `max_derived` bounds each RUN, not the engine lifetime —
/// two runs each under the limit must both succeed.
#[tokio::test]
async fn max_derived_is_per_run_not_cumulative() {
    let graph = seed_graph(
        &["a", "b", "x", "c", "d", "y"],
        vec![edge("a", "supplier_of", "b"), edge("b", "listed_on", "x")],
    )
    .await;
    let tags: Arc<dyn TagEngine> = Arc::new(InMemoryTagEngine::default());
    let mut engine = InferEngine::with_limits(RunLimits {
        max_iterations: 32,
        max_derived: 1,
    });
    engine
        .hot_reload(vec![InferRule::DeriveEdge {
            rule_id: "exposure".to_string(),
            stratum: 0,
            when: pattern(&["supplier_of", "listed_on"]),
            derived_type: "exposed_to".to_string(),
        }])
        .expect("rules");
    assert_eq!(
        engine
            .run_full(&graph, &tags, NOW)
            .await
            .expect("first run")
            .derived_edges,
        1
    );
    // New base facts arrive; the second run derives one MORE fact — under
    // the per-run limit even though the lifetime total is now 2.
    graph
        .upsert_edges(vec![
            edge("c", "supplier_of", "d"),
            edge("d", "listed_on", "y"),
        ])
        .await
        .expect("more edges");
    assert_eq!(
        engine
            .run_full(&graph, &tags, NOW)
            .await
            .expect("second run must not trip a cumulative limit")
            .derived_edges,
        1
    );
}
