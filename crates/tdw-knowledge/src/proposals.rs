//! The agent writeback gate (knowledge-system B9).
//!
//! Agents never write into the knowledge substrate directly. Every write is
//! a [`Proposal`] moving `Draft -> Validated -> Ready`:
//!
//! 1. **Admission** — the proposing agent's [`Adaptivity`] must be at least
//!    `Learning`; below that the submission is rejected outright.
//! 2. **`Draft -> Validated`** — automated validators run on enqueue (shape
//!    grammars, endpoints exist, tag defined, no re-assertion of an existing
//!    fact, per-agent pending cap). A validator failure rejects loudly.
//! 3. **`Validated -> Ready`** — eval-driven: `promote_for_agent` promotes
//!    the agent's validated proposals iff its eval `pass_rate` meets the
//!    ready threshold (default 0.8). Human approval (`approve`) is the
//!    alternative path. `SelfModifying` agents are exempt from HUMAN review
//!    only — never from evals.
//! 4. **Materialization** — only `Ready` proposals write into the graph/tag
//!    engines, with provenance `agent:<id>;proposal:<pid>` (edges carry
//!    [`Provenance::Agent`] with `gated: true`).
//!
//! Net: `Learning` grants the right to PROPOSE; passing evals (or a human)
//! grants the right to LAND.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tdw_core::{GraphEdge, GraphEngine, GraphNode, Provenance, validate_graph_edge};
use tdw_tags::{TagAssignment, TagDefinition, TagEngine, validate_definition_shape};
use tdw_taxonomy::{Adaptivity, EntityKind, ValidationStatus};

use crate::{KnowledgeError, Result};

/// Default eval pass-rate needed for `Validated -> Ready` promotion.
pub const READY_THRESHOLD: f64 = 0.8;
/// Default cap on one agent's pending (non-materialized) proposals.
pub const PER_AGENT_PENDING_CAP: usize = 32;
/// Annotation note ceiling (notes flow into graph props and agent context).
pub const MAX_NOTE_CHARS: usize = 4096;

/// What an agent wants to write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposalKind {
    /// A new graph edge.
    Edge {
        from: String,
        to: String,
        rel: String,
    },
    /// A time-bounded tag assignment.
    TagAssign { entity_id: String, tag_id: String },
    /// A new taxonomy node.
    TagDefine {
        tag_id: String,
        parent: Option<String>,
    },
    /// A free-text note attached to an entity (stored as an annotation node
    /// + `annotated_by` edge).
    Annotation { entity_id: String, note: String },
}

/// One gated write request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// Queue-assigned id (`p<seq>`).
    pub id: String,
    pub kind: ProposalKind,
    pub agent_id: String,
    pub status: ValidationStatus,
    /// Set when rejected (human or validator); a rejected proposal never
    /// promotes or materializes.
    #[serde(default)]
    pub rejected: Option<String>,
    /// True once written into the engines.
    #[serde(default)]
    pub materialized: bool,
    /// Audit trail: every transition with its `now` date.
    pub history: Vec<String>,
}

impl Proposal {
    /// Pending = occupies the agent's queue capacity.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        !self.materialized && self.rejected.is_none()
    }
}

/// Report of one materialization sweep.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializeReport {
    /// Proposal ids written into the engines.
    pub materialized: Vec<String>,
}

/// The gated proposal queue. In-process state; persistence (when needed)
/// goes through serde like the B5 manifest — the daemon owns paths.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProposalQueue {
    proposals: BTreeMap<String, Proposal>,
    next_id: u64,
    #[serde(default)]
    ready_threshold: Option<f64>,
    #[serde(default)]
    pending_cap: Option<usize>,
}

impl ProposalQueue {
    /// Override the eval promotion threshold (config seam).
    #[must_use]
    pub const fn with_ready_threshold(mut self, threshold: f64) -> Self {
        self.ready_threshold = Some(threshold);
        self
    }

    fn threshold(&self) -> f64 {
        self.ready_threshold.unwrap_or(READY_THRESHOLD)
    }

    fn cap(&self) -> usize {
        self.pending_cap.unwrap_or(PER_AGENT_PENDING_CAP)
    }

    /// Submit a proposal through the FULL gate: admission (`Adaptivity >=
    /// Learning`), per-agent pending cap, and the automated validators.
    /// Success leaves the proposal `Validated`; any failure is a loud error
    /// and nothing is enqueued.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::Tag`]/[`KnowledgeError::Storage`]-shaped
    /// errors describing the failing gate step.
    pub async fn submit(
        &mut self,
        agent_id: &str,
        adaptivity: Adaptivity,
        kind: ProposalKind,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
    ) -> Result<Proposal> {
        if agent_id.trim().is_empty() || agent_id.chars().any(char::is_control) {
            return Err(KnowledgeError::Storage("invalid agent id".to_string()));
        }
        // 1. Admission: below Learning never writes (mirrors run_eval_at's
        // feedback gate).
        if adaptivity < Adaptivity::Learning {
            return Err(KnowledgeError::Storage(format!(
                "agent {agent_id:?} has adaptivity {adaptivity:?} — below Learning, writes are \
                 not admitted"
            )));
        }
        // 2. Per-agent pending cap.
        let pending = self
            .proposals
            .values()
            .filter(|proposal| proposal.agent_id == agent_id && proposal.is_pending())
            .count();
        if pending >= self.cap() {
            return Err(KnowledgeError::Storage(format!(
                "agent {agent_id:?} has {pending} pending proposals (cap {}) — promote, \
                 materialize, or reject before submitting more",
                self.cap()
            )));
        }
        // 3. Automated validators (Draft -> Validated on enqueue).
        validate_kind(&kind, graph, tags).await?;

        self.next_id += 1;
        let id = format!("p{}", self.next_id);
        let proposal = Proposal {
            id: id.clone(),
            kind,
            agent_id: agent_id.to_string(),
            status: ValidationStatus::Validated,
            rejected: None,
            materialized: false,
            history: vec![
                format!("{now} draft by {agent_id}"),
                format!("{now} validated (automated validators)"),
            ],
        };
        self.proposals.insert(id, proposal.clone());
        Ok(proposal)
    }

    /// Eval-driven promotion: every `Validated` proposal by `agent_id` moves
    /// to `Ready` iff `pass_rate` meets the threshold. Returns the promoted
    /// ids (empty when the rate falls short — that is not an error; the
    /// proposals simply wait).
    pub fn promote_for_agent(&mut self, agent_id: &str, pass_rate: f64, now: &str) -> Vec<String> {
        if pass_rate < self.threshold() {
            return Vec::new();
        }
        let mut promoted = Vec::new();
        for proposal in self.proposals.values_mut() {
            if proposal.agent_id == agent_id
                && proposal.status == ValidationStatus::Validated
                && proposal.is_pending()
            {
                proposal.status = ValidationStatus::Ready;
                proposal
                    .history
                    .push(format!("{now} ready (eval pass_rate {pass_rate:.2})"));
                promoted.push(proposal.id.clone());
            }
        }
        promoted
    }

    /// Human approval: the alternative `Validated -> Ready` path.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown ids or proposals not in `Validated`.
    pub fn approve(&mut self, proposal_id: &str, approved_by: &str, now: &str) -> Result<()> {
        let proposal = self
            .proposals
            .get_mut(proposal_id)
            .ok_or_else(|| KnowledgeError::Storage(format!("unknown proposal {proposal_id:?}")))?;
        if proposal.rejected.is_some() || proposal.status != ValidationStatus::Validated {
            return Err(KnowledgeError::Storage(format!(
                "proposal {proposal_id:?} is not awaiting approval"
            )));
        }
        proposal.status = ValidationStatus::Ready;
        proposal
            .history
            .push(format!("{now} ready (approved by {approved_by})"));
        Ok(())
    }

    /// Reject a pending proposal with a reason. Terminal.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown ids or already-materialized proposals.
    pub fn reject(&mut self, proposal_id: &str, reason: &str, now: &str) -> Result<()> {
        let proposal = self
            .proposals
            .get_mut(proposal_id)
            .ok_or_else(|| KnowledgeError::Storage(format!("unknown proposal {proposal_id:?}")))?;
        if proposal.materialized {
            return Err(KnowledgeError::Storage(format!(
                "proposal {proposal_id:?} is already materialized"
            )));
        }
        proposal.rejected = Some(reason.to_string());
        proposal.history.push(format!("{now} rejected: {reason}"));
        Ok(())
    }

    /// Write every `Ready` proposal into the engines with provenance
    /// `agent:<id>;proposal:<pid>` and mark it materialized. Only `Ready`
    /// proposals write — pending facts are invisible to reads by default.
    ///
    /// # Errors
    ///
    /// Returns the first failing engine error; already-written proposals in
    /// this sweep stay materialized (the write is per-proposal).
    pub async fn materialize_ready(
        &mut self,
        graph: &Arc<dyn GraphEngine>,
        tags: &Arc<dyn TagEngine>,
        now: &str,
    ) -> Result<MaterializeReport> {
        let ready: Vec<String> = self
            .proposals
            .values()
            .filter(|proposal| proposal.status == ValidationStatus::Ready && proposal.is_pending())
            .map(|proposal| proposal.id.clone())
            .collect();
        let mut report = MaterializeReport::default();
        for id in ready {
            let proposal = self.proposals.get(&id).cloned().ok_or_else(|| {
                KnowledgeError::Storage(format!("proposal {id:?} vanished mid-sweep"))
            })?;
            write_proposal(&proposal, graph, tags, now).await?;
            if let Some(proposal) = self.proposals.get_mut(&id) {
                proposal.materialized = true;
                proposal.history.push(format!("{now} materialized"));
            }
            report.materialized.push(id);
        }
        Ok(report)
    }

    /// Proposals, optionally filtered by agent. Sorted by id.
    #[must_use]
    pub fn list(&self, agent_id: Option<&str>) -> Vec<&Proposal> {
        self.proposals
            .values()
            .filter(|proposal| agent_id.is_none_or(|agent| proposal.agent_id == agent))
            .collect()
    }

    /// One proposal by id.
    #[must_use]
    pub fn get(&self, proposal_id: &str) -> Option<&Proposal> {
        self.proposals.get(proposal_id)
    }
}

/// The automated validators (gate step 2). Every check is loud.
async fn validate_kind(
    kind: &ProposalKind,
    graph: &Arc<dyn GraphEngine>,
    tags: &Arc<dyn TagEngine>,
) -> Result<()> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    match kind {
        ProposalKind::Edge { from, to, rel } => {
            // Shape validity via the shared edge grammar (probe edge).
            let probe = GraphEdge {
                from: from.clone(),
                to: to.clone(),
                rel: rel.clone(),
                props: serde_json::Value::Null,
                provenance: Provenance::Agent {
                    agent_id: "probe".to_string(),
                    gated: true,
                },
                valid_from: None,
                valid_to: None,
            };
            validate_graph_edge(&probe).map_err(storage)?;
            // Endpoints must exist (no stub creation through the gate).
            for endpoint in [from, to] {
                if graph.node(endpoint).await.map_err(storage)?.is_none() {
                    return Err(KnowledgeError::Storage(format!(
                        "edge endpoint {endpoint:?} does not exist"
                    )));
                }
            }
            // No re-assertion of an existing fact (B7 posture: agent writes
            // never clobber or duplicate existing triples).
            let mut offset = 0;
            loop {
                let page = graph.edges(Some(rel), offset, 256).await.map_err(storage)?;
                if page.is_empty() {
                    break;
                }
                offset += page.len();
                if page.iter().any(|edge| &edge.from == from && &edge.to == to) {
                    return Err(KnowledgeError::Storage(format!(
                        "edge {from} -{rel}-> {to} already exists"
                    )));
                }
            }
            Ok(())
        }
        ProposalKind::TagAssign { entity_id, tag_id } => {
            // The tag must be DEFINED — the gate never creates taxonomy as a
            // side effect of assignment.
            if !tag_exists(tags, tag_id).await? {
                return Err(KnowledgeError::Tag(format!("unknown tag {tag_id:?}")));
            }
            if graph.node(entity_id).await.map_err(storage)?.is_none() {
                return Err(KnowledgeError::Storage(format!(
                    "entity {entity_id:?} does not exist"
                )));
            }
            Ok(())
        }
        ProposalKind::TagDefine { tag_id, parent } => {
            validate_definition_shape(&TagDefinition {
                tag_id: tag_id.clone(),
                parent: parent.clone(),
                ttl_days: None,
            })
            .map_err(|error| KnowledgeError::Tag(error.to_string()))?;
            if let Some(parent) = parent
                && !tag_exists(tags, parent).await?
            {
                return Err(KnowledgeError::Tag(format!(
                    "unknown parent tag {parent:?}"
                )));
            }
            if tag_exists(tags, tag_id).await? {
                return Err(KnowledgeError::Tag(format!(
                    "tag {tag_id:?} is already defined"
                )));
            }
            Ok(())
        }
        ProposalKind::Annotation { entity_id, note } => {
            if note.trim().is_empty() || note.chars().count() > MAX_NOTE_CHARS {
                return Err(KnowledgeError::Storage(format!(
                    "annotation note must be non-empty and at most {MAX_NOTE_CHARS} characters"
                )));
            }
            if note
                .chars()
                .any(|character| character.is_control() && character != '\n')
            {
                return Err(KnowledgeError::Storage(
                    "annotation note must not contain control characters".to_string(),
                ));
            }
            if graph.node(entity_id).await.map_err(storage)?.is_none() {
                return Err(KnowledgeError::Storage(format!(
                    "entity {entity_id:?} does not exist"
                )));
            }
            Ok(())
        }
    }
}

/// Tag existence via the engine contract's `is_defined`.
async fn tag_exists(tags: &Arc<dyn TagEngine>, tag_id: &str) -> Result<bool> {
    tags.is_defined(tag_id)
        .await
        .map_err(|error| KnowledgeError::Tag(error.to_string()))
}

/// Write one Ready proposal into the engines.
async fn write_proposal(
    proposal: &Proposal,
    graph: &Arc<dyn GraphEngine>,
    tags: &Arc<dyn TagEngine>,
    now: &str,
) -> Result<()> {
    let storage = |error: tdw_core::Error| KnowledgeError::Storage(error.to_string());
    let provenance_text = format!("agent:{};proposal:{}", proposal.agent_id, proposal.id);
    match &proposal.kind {
        ProposalKind::Edge { from, to, rel } => graph
            .upsert_edges(vec![GraphEdge {
                from: from.clone(),
                to: to.clone(),
                rel: rel.clone(),
                props: serde_json::json!({ "proposal": proposal.id }),
                provenance: Provenance::Agent {
                    agent_id: proposal.agent_id.clone(),
                    gated: true,
                },
                valid_from: None,
                valid_to: None,
            }])
            .await
            .map_err(storage),
        ProposalKind::TagAssign { entity_id, tag_id } => tags
            .assign(TagAssignment {
                entity_id: entity_id.clone(),
                tag_id: tag_id.clone(),
                assigned_at: now.to_string(),
                expires_at: None,
                provenance: provenance_text,
            })
            .await
            .map_err(|error| KnowledgeError::Tag(error.to_string())),
        ProposalKind::TagDefine { tag_id, parent } => tags
            .define(TagDefinition {
                tag_id: tag_id.clone(),
                parent: parent.clone(),
                ttl_days: None,
            })
            .await
            .map_err(|error| KnowledgeError::Tag(error.to_string())),
        ProposalKind::Annotation { entity_id, note } => {
            let annotation_id = format!("annotation:{}", proposal.id);
            graph
                .upsert_nodes(vec![GraphNode {
                    id: annotation_id.clone(),
                    kind: EntityKind::Document,
                    label: format!("annotation {}", proposal.id),
                    aliases: Vec::new(),
                    props: serde_json::json!({
                        "note": note,
                        "agent_id": proposal.agent_id,
                        "proposal": proposal.id,
                    }),
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .map_err(storage)?;
            graph
                .upsert_edges(vec![GraphEdge {
                    from: entity_id.clone(),
                    to: annotation_id,
                    rel: "annotated_by".to_string(),
                    props: serde_json::Value::Null,
                    provenance: Provenance::Agent {
                        agent_id: proposal.agent_id.clone(),
                        gated: true,
                    },
                    valid_from: None,
                    valid_to: None,
                }])
                .await
                .map_err(storage)
        }
    }
}
