//! The write-back step (the FinX Partner design §1.3 step 5, §3).
//!
//! Every turn writes back *autonomously within the gate*: an episodic memory of
//! the turn, candidate finding(s), a feedback hook, and any knowledge-graph
//! mutation as a gated [`ProposalQueue::submit`]. This module owns the gated KG
//! mutation — the one write that must pass the B9 / Adaptivity gate. Episodic +
//! finding + feedback are routed through the MCP `tdw.kg.remember` /
//! `tdw.kg.finding` / `tdw.kg.feedback` handlers by the surface adapters; here
//! we own the proposal submission so the gate lives in exactly one place.
//!
//! Audit-only autonomy (the FinX Partner design directive 2): the proposal is *submitted*
//! through the gate; it does not block on a human. The gate itself
//! ([`ProposalQueue::submit`] refusing below [`Adaptivity::Learning`]) plus
//! reversibility (`ProposalKind::Forget`) are the safety net.

use std::sync::Arc;

use tdw_core::GraphEngine;
use tdw_knowledge::proposals::{Proposal, ProposalKind, ProposalQueue};
use tdw_tags::TagEngine;
use tdw_taxonomy::Adaptivity;

use crate::principal::Principal;

/// Submit a knowledge-graph mutation through the write-back gate.
///
/// This is the single seam that enforces "no behavior change reaches the graph
/// except past the gate" (the FinX Partner design §3). It threads the principal's
/// [`Adaptivity`] into [`ProposalQueue::submit`], which **refuses** the write
/// when the adaptivity is below [`Adaptivity::Learning`] — the assertion the
/// W2.5 gate test pins.
///
/// # Errors
///
/// Returns the queue's error (a `tdw_knowledge::KnowledgeError`) when admission
/// fails — most importantly when the principal's trust dial is below
/// `Learning`, in which case nothing is written.
pub async fn submit_kg_mutation(
    queue: &mut ProposalQueue,
    principal: &Principal,
    kind: ProposalKind,
    graph: &Arc<dyn GraphEngine>,
    tags: &Arc<dyn TagEngine>,
    now: &str,
) -> tdw_knowledge::Result<Proposal> {
    queue
        .submit(
            &principal.agent_id,
            principal.trust.adaptivity,
            kind,
            graph,
            tags,
            now,
        )
        .await
}

/// Whether the principal's trust dial admits autonomous KG writes at all.
///
/// A cheap pre-check mirroring the gate's admission rule: writes are admitted
/// only at [`Adaptivity::Learning`] or above. Surfaces use this to decide
/// whether to attempt a write-back without round-tripping the queue.
#[must_use]
pub fn writes_admitted(principal: &Principal) -> bool {
    principal.trust.adaptivity >= Adaptivity::Learning
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::principal::TrustContext;
    use tdw_storage_graph::InMemoryGraphEngine;
    use tdw_tags::InMemoryTagEngine;

    fn engines() -> (Arc<dyn GraphEngine>, Arc<dyn TagEngine>) {
        (
            Arc::new(InMemoryGraphEngine::default()),
            Arc::new(InMemoryTagEngine::default()),
        )
    }

    fn principal_at(adaptivity: Adaptivity) -> Principal {
        Principal::new("sess", "agent:partner").with_trust(TrustContext {
            adaptivity,
            retrieval_floor: 0.0,
        })
    }

    #[tokio::test]
    async fn gate_refuses_writes_below_learning() {
        let (graph, tags) = engines();
        let mut queue = ProposalQueue::default();
        let principal = principal_at(Adaptivity::Configured);
        assert!(!writes_admitted(&principal));

        let result = submit_kg_mutation(
            &mut queue,
            &principal,
            ProposalKind::Annotation {
                entity_id: "AAPL".to_string(),
                note: "noticed strong close".to_string(),
            },
            &graph,
            &tags,
            "2026-06-14",
        )
        .await;
        assert!(
            result.is_err(),
            "a below-Learning principal must be refused by the gate"
        );
    }

    #[tokio::test]
    async fn gate_admits_writes_at_learning() {
        let (graph, tags) = engines();
        // Seed the entity the annotation attaches to so the proposal passes the
        // queue's entity-existence validator — we are asserting the *adaptivity
        // gate* admits a Learning principal, not exercising a missing-entity path.
        graph
            .upsert_nodes(vec![tdw_core::GraphNode {
                id: "AAPL".to_string(),
                kind: tdw_taxonomy::EntityKind::Instrument,
                label: "Apple Inc.".to_string(),
                aliases: vec![],
                props: serde_json::Value::Null,
                valid_from: None,
                valid_to: None,
            }])
            .await
            .expect("seed entity");
        let mut queue = ProposalQueue::default();
        let principal = principal_at(Adaptivity::Learning);
        assert!(writes_admitted(&principal));

        let result = submit_kg_mutation(
            &mut queue,
            &principal,
            ProposalKind::Annotation {
                entity_id: "AAPL".to_string(),
                note: "noticed strong close".to_string(),
            },
            &graph,
            &tags,
            "2026-06-14",
        )
        .await;
        let proposal = result.expect("a Learning principal is admitted by the gate");
        assert_eq!(proposal.agent_id, "agent:partner");
        assert!(proposal.is_pending());
    }
}
