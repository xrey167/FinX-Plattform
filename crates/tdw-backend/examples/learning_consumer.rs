//! End-to-end **learning** consumer embedding `tdw-backend`.
//!
//! This example is the capstone proof for the agent-learning mission: a single
//! process that drives all three learning capabilities end to end, using
//! [`tdw_backend::prelude`] as the only backend import:
//!
//! * **Knowledge** — index a [`KnowledgeDocument`] into the async knowledge index
//!   and retrieve it by semantic search. Offline here (the deterministic in-memory
//!   embedder + vector engine); the durable Qdrant path is env-gated (it needs the
//!   `real-qdrant` feature **and** `TDW_QDRANT_URL`) and out of scope for this
//!   offline example.
//! * **Memory consolidation** — upsert a `Working`-tier [`Memory`] and run one
//!   consolidation pass: the loop returns a [`ConsolidationAction`] that expires the
//!   ephemeral buffer (ttl 0) on the first tick, and `list_memories` no longer
//!   contains it.
//! * **Eval feedback (Adaptivity-gated)** — register an [`AgentCard`] with one
//!   `Learning` skill and one `Configured` skill, run an eval whose case expects a
//!   `ContentRef` URI the agent surfaces, and observe that only the `Learning`
//!   skill accrues quality state (the gate skips the `Configured` skill).
//!
//! It is fully offline and deterministic — no network, no Docker — and must run to
//! completion without hanging. The construction shapes for the entities reuse the
//! crates' own test fixtures (mirroring `examples/trading_consumer.rs`, which also
//! reaches for `tdw_*` crate paths to build entities).
//!
//! Run it with:
//!
//! ```text
//! cargo run --example learning_consumer -p tdw-backend --target-dir target
//! ```

// The prelude is the ONLY backend import: this proves the prelude surface is
// sufficient to drive the full learning loop from a real consumer.
use tdw_backend::prelude::*;

use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("== tdw-backend learning consumer ==");

    let backend = Backend::in_memory_for_tests().await;

    knowledge_flow(&backend).await?;
    memory_flow(&backend).await?;
    eval_flow()?;

    println!("[done] all three learning capabilities exercised offline; clean exit.");
    Ok(())
}

/// Capability 1 — durable + semantic knowledge (in-memory here; durable under
/// `real-qdrant` + `TDW_QDRANT_URL`).
async fn knowledge_flow(backend: &Backend) -> Result<(), Box<dyn Error>> {
    // Construction mirrors `tdw-knowledge`'s own indexing test fixture.
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
        .await?;
    println!(
        "[knowledge] indexed 1 document (in-memory embedder; durable under real-qdrant + TDW_QDRANT_URL)"
    );

    let hits = backend.knowledge_search("AAPL momentum", 1).await?;
    let top = hits.first().ok_or("knowledge search returned no hit")?;
    println!(
        "[knowledge] search top hit = {} (entity {})",
        top.id, top.entity_id
    );
    Ok(())
}

/// Capability 2 — the memory consolidation loop expires an ephemeral working
/// buffer on the first pass.
async fn memory_flow(backend: &Backend) -> Result<(), Box<dyn Error>> {
    backend
        .upsert_memory(working_memory("scratch-buffer"))
        .await?;
    println!(
        "[memory] upserted a Working-tier memory; store now holds {}",
        backend.list_memories().await.len()
    );

    // A Working buffer has ttl 0, so it expires on the first consolidation tick
    // regardless of the wall clock — deterministic offline.
    let actions = backend.consolidate_now().await?;
    for action in &actions {
        println!("[memory] consolidation action = {action:?}");
    }

    let remaining = backend.list_memories().await;
    println!(
        "[memory] after consolidation, store holds {} memories (buffer expired = {})",
        remaining.len(),
        remaining
            .iter()
            .all(|m| m.meta.base.name != "scratch-buffer")
    );
    Ok(())
}

/// Capability 3 — gated eval feedback updates the `Learning` skill while skipping
/// the `Configured` one.
fn eval_flow() -> Result<(), Box<dyn Error>> {
    let cfg = BackendConfig::default();
    let mut agent = AgentBackend::from_config(&cfg)?;

    agent.upsert_agent(learning_agent_card());
    println!("[eval] registered agent 'market-researcher' (Learning + Configured skills)");

    // The case expects the content_ref URI the agent surfaces, so the stub model's
    // echoed grounding context passes it -> pass_rate 1.0.
    let outcome = agent.run_eval(tdw_agent::EvalRunRequest {
        run_id: "eval-learning-consumer".to_string(),
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
    });
    println!(
        "[eval] run '{}' status = {}",
        outcome.run_id, outcome.status
    );

    let card = agent
        .agent("market-researcher")
        .ok_or("agent card must be present after the eval run")?;
    let learning = card.skills.first().ok_or("learning skill present")?;
    let configured = card.skills.get(1).ok_or("configured skill present")?;

    let quality = learning
        .quality
        .as_ref()
        .ok_or("the Learning skill must have accrued quality state")?;
    println!(
        "[eval] Learning skill '{}': runs={}, pass_rate={:?}, disabled={} (gate applied feedback)",
        learning.meta.base.name, quality.runs, quality.pass_rate, quality.disabled
    );
    println!(
        "[eval] Configured skill '{}': quality is None = {} (gate skipped it)",
        configured.meta.base.name,
        configured.quality.is_none()
    );
    Ok(())
}

/// A `Working`-tier [`Memory`] mirroring `tdw-backend`'s own Phase-B memory
/// fixture. Working (ttl 0) expires on the first consolidation pass.
fn working_memory(name: &str) -> Memory {
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
        retention: Retention::Working,
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
/// skill (gate skips), plus the `content_ref` whose URI the eval case expects.
/// Construction mirrors the C2 backend feedback test in `agent/mod.rs`.
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
