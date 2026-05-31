//! End-to-end integration test for the three agent-learning capabilities.
//!
//! The deterministic, offline counterpart to `examples/learning_consumer.rs`:
//! every learning surface reached through [`tdw_backend::prelude`] is asserted.
//!
//! * **Knowledge** — index a [`KnowledgeDocument`] and assert semantic search
//!   returns it.
//! * **Memory consolidation** — a `Working` buffer (ttl 0) is `Expire`d on the
//!   first `consolidate_now` pass and leaves the store; a `ShortTerm` memory whose
//!   `last_consolidated` is seeded far in the past (so its age deterministically
//!   exceeds its 1-day ttl under the live clock) is `Promote`d to `MidTerm`.
//! * **Eval feedback** — an [`AgentCard`] with one `Learning` and one `Configured`
//!   skill is evaluated; only the `Learning` skill accrues quality state (the
//!   Adaptivity gate skips the `Configured` skill).
//!
//! Offline (no Docker/network) and deterministic: the memory ages use seeded
//! anchors, not the wall clock, and nothing blocks indefinitely.

use tdw_backend::prelude::*;

// --- Capability 1: knowledge ------------------------------------------------

#[tokio::test]
async fn knowledge_index_then_search_returns_the_document() {
    let backend = Backend::in_memory_for_tests().await;

    backend
        .knowledge_index(KnowledgeDocument {
            id: "doc-aapl-momentum".to_string(),
            body: "AAPL equity momentum desk research note".to_string(),
            entity: Entity {
                entity_id: "instrument:AAPL".to_string(),
                kind: tdw_kg::EntityKind::Instrument,
                label: "Apple".to_string(),
                aliases: vec!["AAPL".to_string()],
            },
            tags: vec!["asset:equity".to_string()],
        })
        .await
        .expect("knowledge index should succeed");

    let hits = backend
        .knowledge_search("AAPL momentum", 1)
        .await
        .expect("knowledge search should succeed");

    assert_eq!(hits.len(), 1, "search must surface exactly the one indexed doc");
    assert_eq!(hits[0].id, "doc-aapl-momentum");
    assert_eq!(hits[0].entity_id, "instrument:AAPL");
}

// --- Capability 2: memory consolidation -------------------------------------

#[tokio::test]
async fn working_memory_expires_on_first_consolidation_pass() {
    let backend = Backend::in_memory_for_tests().await;

    // A Working buffer has ttl 0, so it expires on the first tick regardless of
    // the wall clock — deterministic offline.
    backend
        .upsert_memory(memory("scratch-buffer", Retention::Working))
        .await
        .expect("upsert working memory");
    assert_eq!(backend.list_memories().await.len(), 1);

    let actions = backend.consolidate_now().await.expect("consolidate");
    assert_eq!(actions.len(), 1, "the working buffer yields exactly one action");
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, ConsolidationAction::Expire { name } if name == "scratch-buffer")),
        "the Working buffer must be Expired: {actions:?}"
    );
    assert!(
        backend
            .list_memories()
            .await
            .iter()
            .all(|m| m.meta.base.name != "scratch-buffer"),
        "the expired buffer must no longer be listed"
    );
}

#[tokio::test]
async fn aged_short_term_memory_promotes_to_mid_term() {
    let backend = Backend::in_memory_for_tests().await;

    // Seed a ShortTerm memory whose `last_consolidated` is far in the past. The
    // store's `upsert_at` preserves a preset anchor (it only stamps `now` when the
    // anchor is None), so under the live clock the memory's age vastly exceeds its
    // 1-day ttl and consolidation deterministically promotes it to MidTerm.
    let mut seeded = memory("aged-note", Retention::ShortTerm);
    seeded.last_consolidated = Some("2000-01-01T00:00:00+00:00".to_string());
    {
        let store = backend.memory_store();
        let mut guard = store.lock().await;
        guard
            .upsert_at(seeded, "2000-01-01T00:00:00+00:00")
            .expect("seed aged short-term memory");
    }

    let actions = backend.consolidate_now().await.expect("consolidate");
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, ConsolidationAction::Promote { name, to, .. } if name == "aged-note" && *to == Retention::MidTerm)),
        "the aged ShortTerm memory must be Promoted to MidTerm: {actions:?}"
    );

    let promoted = backend.list_memories().await;
    let note = promoted
        .iter()
        .find(|m| m.meta.base.name == "aged-note")
        .expect("the promoted memory remains in the store");
    assert_eq!(
        note.retention,
        Retention::MidTerm,
        "the surviving memory consolidated up one tier"
    );
}

// --- Capability 3: gated eval feedback --------------------------------------

#[test]
fn eval_feedback_updates_learning_skill_and_skips_configured_skill() {
    let cfg = BackendConfig::default();
    let mut agent = AgentBackend::from_config(&cfg).expect("agent backend should build");

    agent.upsert_agent(learning_agent_card());

    // The case expects the content_ref URI the agent surfaces, so the stub model's
    // echoed grounding context passes it -> pass_rate 1.0 (>= the 0.5 threshold,
    // so the Learning skill stays enabled). A fixed `now` keeps it deterministic.
    let outcome = agent.run_eval_at(
        tdw_agent::EvalRunRequest {
            run_id: "eval-learning-e2e".to_string(),
            agent_id: "market-researcher".to_string(),
            dataset_id: "golden-market-notes".to_string(),
            cases: vec![tdw_agent::EvalCase {
                case_id: "case-1".to_string(),
                prompt: "Summarize AAPL".to_string(),
                expected_refs: vec![tdw_agent::ContentRef {
                    uri: "tdw://docs/research-template".to_string(),
                    kind: tdw_agent::ContentKind::Prompt,
                    checksum: None,
                    tags: Vec::new(),
                }],
            }],
        },
        "2026-05-31T00:00:00+00:00",
    );
    assert_eq!(outcome.status, "success");

    let card = agent.agent("market-researcher").expect("card present");
    let learning = &card.skills[0];
    let configured = &card.skills[1];

    // The Learning skill received gated feedback: runs=1, pass_rate=1.0, enabled.
    let quality = learning
        .quality
        .as_ref()
        .expect("the Learning skill must have accrued quality state");
    assert_eq!(quality.runs, 1, "exactly one eval fed back");
    assert_eq!(quality.pass_rate, Some(1.0), "the case passed");
    assert!(!quality.disabled, "pass_rate >= threshold keeps the skill enabled");
    assert_eq!(
        quality.last_eval.as_deref(),
        Some("2026-05-31T00:00:00+00:00")
    );

    // The Configured skill was gate-skipped: its quality stays None.
    assert!(
        configured.quality.is_none(),
        "the Adaptivity gate must skip the Configured skill"
    );
}

// --- Fixtures ---------------------------------------------------------------

/// A [`Memory`] of the given `retention`, mirroring `tdw-backend`'s own Phase-B
/// memory fixture.
fn memory(name: &str, retention: Retention) -> Memory {
    use tdw_agent::{
        Adaptivity, DataFacets, EntityMeta, Materialization, Origin, Plane, Source, Tier,
    };
    Memory {
        meta: EntityMeta::new(
            name,
            name,
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::SelfModifying,
            false,
        ),
        retention,
        last_consolidated: None,
        source_entries: Vec::new(),
        facets: DataFacets {
            plane: Plane::Agent,
            materialization: Materialization::Materialized,
            as_of: None,
            validation: None,
        },
    }
}

/// An [`AgentCard`] with one `Learning` skill (gate passes) and one `Configured`
/// skill (gate skips), plus the `content_ref` the eval case expects. Construction
/// mirrors the C2 backend feedback test in `agent/mod.rs`.
fn learning_agent_card() -> AgentCard {
    use tdw_agent::{
        Adaptivity, AgentSkill, ContentKind, ContentRef, EntityMeta, Origin, Source, Tier,
    };

    let skill = |name: &str, adaptivity: Adaptivity| AgentSkill {
        meta: EntityMeta::new(
            name,
            name,
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            adaptivity,
            false,
        )
        .with_title(name)
        .with_description("A skill."),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        quality: None,
    };

    AgentCard {
        meta: EntityMeta::new(
            "market-researcher",
            "market-researcher",
            "0.1.0",
            Origin {
                tier: Tier::Domain,
                source: Source::Internal,
            },
            Adaptivity::Learning,
            true,
        )
        .with_title("Market Researcher")
        .with_description("Generates evidence-backed notes."),
        skills: vec![
            skill("research.note", Adaptivity::Learning),
            skill("research.summary", Adaptivity::Configured),
        ],
        content_refs: vec![ContentRef {
            uri: "tdw://docs/research-template".to_string(),
            kind: ContentKind::Prompt,
            checksum: None,
            tags: Vec::new(),
        }],
        endpoint: Some("mcp://tdw/agents/market-researcher".to_string()),
    }
}
