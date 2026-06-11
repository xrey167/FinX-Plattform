//! End-to-end test for the feedback-store host-injection bridge.
//!
//! Verifies the contract documented in [`tdw_agent_store::feedback`]: when an
//! embedding host constructs both an [`AgentBackend`] and a [`Backend`] in the
//! same process and wires them with the same `Arc<Mutex<RetrievalFeedbackStore>>`
//! via [`Backend::with_feedback_store`], an event appended into that store is
//! visible to [`Backend::consolidate_now_at`] and actually changes the
//! consolidation plan (recency credit applied).
//!
//! The MCP JSON-RPC append path is exercised separately in
//! `crates/tdw-mcp/tests/knowledge_feedback_tools.rs`. This test focuses on
//! the backend bridging contract: one Arc, two facades, recency credit flows
//! end-to-end.
//!
//! Standalone `tdw-mcp` entrypoints (stdio, Streamable HTTP) have no
//! co-resident `Backend`; bridging those to a daemon's consolidation loop is
//! F1 work — the same deferral B8/B9 made for `KnowledgeRuntime` daemon hosting.

use std::sync::Arc;

use tdw_agent::{
    Adaptivity, DataFacets, EntityMeta, Materialization, Memory, Origin, Plane, Retention, Source,
    Tier,
};
use tdw_agent_store::{RetrievalEvent, RetrievalFeedbackStore};
use tdw_backend::prelude::*;
use tdw_knowledge::runtime::KnowledgeVersions;
use tokio::sync::Mutex;

/// Build a [`Memory`] whose `last_consolidated` is exactly 2 days before the
/// injected `now` ("2026-06-11"). `ShortTerm` TTL is 1 day, so `raw_age=2 > 1`
/// → the base planner would promote it. A feedback event recorded AT `now`
/// gives `credit = raw_age.saturating_sub(0) = 2`, making
/// `effective_age = 2.saturating_sub(2) = 0 < ttl(1)` → no action.
fn aged_memory(name: &str) -> Memory {
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
        retention: Retention::ShortTerm,
        // 2 days before the injected `now` — past the 1-day TTL.
        last_consolidated: Some("2026-06-09T00:00:00Z".to_string()),
        source_entries: Vec::new(),
        facets: DataFacets {
            plane: Plane::Agent,
            materialization: Materialization::Materialized,
            as_of: None,
            validation: None,
        },
    }
}

/// Build a minimal [`RetrievalEvent`] whose `agent_id` matches `memory_name`
/// so the recency-credit lookup hits.
fn used_event(memory_name: &str, recorded_at: &str) -> RetrievalEvent {
    RetrievalEvent {
        agent_id: memory_name.to_string(),
        query_fingerprint: "fp-bridge-e2e".to_string(),
        hit_ids: vec![],
        versions: KnowledgeVersions {
            embedder_model: "hash".to_string(),
            rules_version: None,
            infer_version: None,
        },
        used: true,
        recorded_at: recorded_at.to_string(),
    }
}

/// End-to-end: append a feedback event through the shared `Arc` handle (the
/// same handle `AgentBackend` passes to its embedded `McpServer`), then run
/// `Backend::consolidate_now_at` and assert the recency credit spares the
/// memory that would otherwise be expired without feedback.
///
/// This is the host-injection contract proof:
/// ```ignore
/// let agent   = AgentBackend::from_config(&cfg)?;
/// let handle  = agent.feedback_store_handle(); // owned by AgentBackend
/// let backend = Backend::in_memory_for_tests()
///     .await
///     .with_feedback_store(Arc::clone(&handle));
/// // agent's McpServer and backend.consolidate_now_at now share one store.
/// ```
#[tokio::test]
async fn feedback_appended_through_shared_handle_reaches_consolidation() {
    // --- Host construction: wire both facades to the same feedback store -----
    let cfg = BackendConfig::default();
    let agent = AgentBackend::from_config(&cfg).expect("AgentBackend::from_config");

    // Extract the store handle AgentBackend already wired into its McpServer.
    let shared_handle: Arc<Mutex<RetrievalFeedbackStore>> = agent.feedback_store_handle();

    // Backend accepts the same Arc — from here, appends through either facade
    // are visible to consolidate_now_at.
    let backend = Backend::in_memory_for_tests()
        .await
        .with_feedback_store(Arc::clone(&shared_handle));

    // Injected now: 2 days after last_consolidated → raw_age=2 > ttl(1).
    let now = "2026-06-11T00:00:00Z";
    let memory_name = "agent:sensor";

    // Seed the aged memory.
    backend
        .upsert_memory(aged_memory(memory_name))
        .await
        .expect("upsert memory");

    // --- Baseline: without feedback the memory IS expired --------------------
    let base_actions = backend
        .consolidate_now_at(now)
        .await
        .expect("baseline consolidation");
    assert!(
        base_actions
            .iter()
            .any(|a| format!("{a:?}").contains(memory_name)),
        "baseline (no feedback) must expire the aged memory: {base_actions:?}"
    );

    // Re-seed after baseline consumed/expired it.
    backend
        .upsert_memory(aged_memory(memory_name))
        .await
        .expect("re-upsert memory");

    // --- Append a feedback event through the shared Arc ----------------------
    // Equivalent to the `tdw.kg.feedback` JSON-RPC call going through
    // AgentBackend's embedded McpServer — both write into this same Arc.
    {
        let mut store = shared_handle.lock().await;
        store
            // Recorded AT `now`: days_since=0 → credit=raw_age(2) →
            // effective_age = 2.saturating_sub(2) = 0 < ttl(1) → no action.
            .append(used_event(memory_name, now))
            .expect("append feedback event through shared handle");
    }

    // The backend's own handle must be the same Arc — verify it sees the event.
    assert_eq!(
        backend.feedback_store_handle().lock().await.len(),
        1,
        "backend must see the event appended through the shared handle (same Arc)"
    );

    // --- Bridged consolidation: recency credit must spare the memory ---------
    // effective_age=0 < ttl(1) → no consolidation action at all.
    let bridged_actions = backend
        .consolidate_now_at(now)
        .await
        .expect("bridged consolidation");
    assert!(
        bridged_actions.is_empty(),
        "recency credit (effective_age=0) must suppress all actions: {bridged_actions:?}"
    );
}
