//! Retrieval feedback store — usage statistics for knowledge-system B10.
//!
//! Agents record retrieval interactions via the `tdw.kg.feedback` MCP tool after
//! a search; each record is a [`RetrievalEvent`] keyed by the agent that made the
//! query. The store is **append-only from the agent side** and is NEVER a direct
//! mutation path to the graph, tags, or proposals — it is strictly a stats
//! accumulator.
//!
//! ## Attachment
//!
//! The store is attached at the [`tdw_mcp::McpServer`] level via
//! [`McpServer::with_feedback_store`] (builder) or
//! [`McpServer::set_feedback_store`] (in-place setter). The same `Arc` handle is
//! held by [`tdw_backend::data::Backend`] so `consolidate_now` and the MCP tool
//! share one store instance.
//!
//! ## Bound / eviction policy
//!
//! The store is bounded by two caps (both configurable; defaults are
//! [`DEFAULT_PER_AGENT_CAP`] and [`DEFAULT_GLOBAL_CAP`]):
//!
//! - **Per-agent cap**: at most `per_agent_cap` events per `agent_id`. When the
//!   agent would exceed its cap the *oldest* event for that agent is evicted and
//!   a loud `eprintln!` warning is emitted so the overflow is never invisible.
//! - **Global cap**: at most `global_cap` events across all agents. When the
//!   store would exceed the global cap the single oldest event (across all
//!   agents) is evicted and a warning is emitted.
//!
//! Both evictions are oldest-first (FIFO); the newest record always wins. A
//! `used` flag on [`RetrievalEvent`] lets consolidation weight recently-used
//! entities more highly than stale ones.
//!
//! ## Structural validation at restore time
//!
//! [`RetrievalFeedbackStore::validate_restored`] is called after deserialising
//! from JSON5 (mirroring the B9 [`ProposalQueue::validate_restored`] precedent)
//! and returns an error when any stored event fails its invariants — empty
//! `agent_id`, empty `query_fingerprint`, or a `KnowledgeVersions` triple whose
//! `embedder_model` is empty.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use tdw_knowledge::runtime::KnowledgeVersions;

/// Default per-agent event cap.
pub const DEFAULT_PER_AGENT_CAP: usize = 256;
/// Default global event cap (across all agents).
pub const DEFAULT_GLOBAL_CAP: usize = 4096;

/// A single retrieval interaction recorded by an agent.
///
/// `now` is **always injected** — it is never read from the wall clock here; the
/// live path supplies `chrono::Utc::now().to_rfc3339()`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalEvent {
    /// The submitting agent's id. Grammar: `[A-Za-z0-9:._-]`, max
    /// [`tdw_knowledge::proposals::MAX_AGENT_ID_LEN`] bytes. Validated by
    /// [`tdw_knowledge::proposals::validate_agent_id`] before admission.
    pub agent_id: String,
    /// A short, caller-supplied fingerprint for the query (e.g. a truncated hash
    /// or a canonical normalised string). Non-empty, max 256 bytes.
    pub query_fingerprint: String,
    /// The document / entity ids the agent considered as hits. Bounded to
    /// [`MAX_HIT_IDS`] entries; excess ids are silently truncated at admission.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hit_ids: Vec<String>,
    /// Knowledge version triple the search was executed against.
    pub versions: KnowledgeVersions,
    /// Whether the agent reported the result as helpful / used. `false` means the
    /// agent explicitly signalled the hits were not useful (or the field was
    /// absent — `default` makes old records deserialise correctly).
    #[serde(default)]
    pub used: bool,
    /// RFC 3339 timestamp injected at record time. Never accepted as caller input.
    pub recorded_at: String,
}

/// Maximum number of hit ids stored per event (excess is silently truncated at
/// admission so the store cannot be inflated by a single oversized payload).
pub const MAX_HIT_IDS: usize = 64;

impl RetrievalEvent {
    /// Validate the structural invariants of a single event.
    ///
    /// # Errors
    ///
    /// Returns a human-readable description when any invariant is violated.
    pub fn validate(&self) -> Result<(), String> {
        if self.agent_id.trim().is_empty() {
            return Err("agent_id must not be empty".to_string());
        }
        if self.query_fingerprint.trim().is_empty() {
            return Err("query_fingerprint must not be empty".to_string());
        }
        if self.query_fingerprint.len() > 256 {
            return Err(format!(
                "query_fingerprint is too long ({} bytes, max 256)",
                self.query_fingerprint.len()
            ));
        }
        if self.versions.embedder_model.trim().is_empty() {
            return Err("versions.embedder_model must not be empty".to_string());
        }
        if self.recorded_at.trim().is_empty() {
            return Err("recorded_at must not be empty".to_string());
        }
        Ok(())
    }
}

/// Per-agent usage summary derived from the stored events.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentUsageSummary {
    /// Total recorded events for this agent.
    pub total_events: usize,
    /// Number of events the agent flagged as `used`.
    pub used_count: usize,
    /// How recently (in RFC 3339 max-order) the agent last submitted a `used`
    /// event; `None` when no used event has been recorded.
    pub last_used_at: Option<String>,
}

/// Bounded append-only store for [`RetrievalEvent`] records.
///
/// Per-agent cap: [`DEFAULT_PER_AGENT_CAP`] (configurable).
/// Global cap: [`DEFAULT_GLOBAL_CAP`] (configurable).
///
/// Both caps evict the oldest matching entry (FIFO) and emit a loud warning on
/// overflow so the overflow is never invisible.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalFeedbackStore {
    /// All stored events in insertion order.
    #[serde(default)]
    events: VecDeque<RetrievalEvent>,
    /// Per-agent event cap (defaults to [`DEFAULT_PER_AGENT_CAP`] when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    per_agent_cap: Option<usize>,
    /// Global event cap (defaults to [`DEFAULT_GLOBAL_CAP`] when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    global_cap: Option<usize>,
}

impl RetrievalFeedbackStore {
    /// An empty store with default caps.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the per-agent event cap. Must be called before any `append`.
    #[must_use]
    pub const fn with_per_agent_cap(mut self, cap: usize) -> Self {
        self.per_agent_cap = Some(cap);
        self
    }

    /// Override the global event cap. Must be called before any `append`.
    #[must_use]
    pub const fn with_global_cap(mut self, cap: usize) -> Self {
        self.global_cap = Some(cap);
        self
    }

    fn effective_per_agent_cap(&self) -> usize {
        self.per_agent_cap.unwrap_or(DEFAULT_PER_AGENT_CAP)
    }

    fn effective_global_cap(&self) -> usize {
        self.global_cap.unwrap_or(DEFAULT_GLOBAL_CAP)
    }

    /// Append one [`RetrievalEvent`], enforcing both caps (oldest-evict with
    /// loud warnings on overflow).
    ///
    /// `hit_ids` longer than [`MAX_HIT_IDS`] are silently truncated here, before
    /// the cap check. The event must already pass [`RetrievalEvent::validate`].
    ///
    /// # Errors
    ///
    /// Returns a description when the event fails structural validation.
    pub fn append(&mut self, mut event: RetrievalEvent) -> Result<(), String> {
        event.validate()?;
        // Truncate oversized hit_ids silently at admission.
        event.hit_ids.truncate(MAX_HIT_IDS);

        let per_agent = self.effective_per_agent_cap();
        let global = self.effective_global_cap();

        // Per-agent cap: evict oldest for this agent when the agent would exceed.
        let agent_count = self
            .events
            .iter()
            .filter(|e| e.agent_id == event.agent_id)
            .count();
        if agent_count >= per_agent {
            // Find and remove the oldest (front-most) event for this agent.
            if let Some(pos) = self
                .events
                .iter()
                .position(|e| e.agent_id == event.agent_id)
            {
                self.events.remove(pos);
            }
            eprintln!(
                "tdw-agent-store: RetrievalFeedbackStore per-agent cap ({per_agent}) reached \
                 for agent {:?}; oldest event evicted",
                event.agent_id
            );
        }

        // Global cap: evict oldest across all agents when the store would exceed.
        if self.events.len() >= global {
            self.events.pop_front();
            eprintln!(
                "tdw-agent-store: RetrievalFeedbackStore global cap ({global}) reached; \
                 oldest event evicted"
            );
        }

        self.events.push_back(event);
        Ok(())
    }

    /// Validate structural invariants of all stored events (called after
    /// deserialising from JSON5).
    ///
    /// # Errors
    ///
    /// Returns a human-readable description on the first invalid event.
    pub fn validate_restored(&self) -> Result<(), String> {
        for (idx, event) in self.events.iter().enumerate() {
            event
                .validate()
                .map_err(|error| format!("event[{idx}] invalid: {error}"))?;
        }
        Ok(())
    }

    /// All stored events in insertion order.
    pub fn events(&self) -> impl Iterator<Item = &RetrievalEvent> {
        self.events.iter()
    }

    /// Events for a specific agent, in insertion order.
    pub fn events_for(&self, agent_id: &str) -> impl Iterator<Item = &RetrievalEvent> {
        self.events.iter().filter(move |e| e.agent_id == agent_id)
    }

    /// The number of stored events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the store holds no events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Compute a [`AgentUsageSummary`] for one agent from the stored events.
    ///
    /// Returns a zero-valued summary when the agent has no recorded events.
    #[must_use]
    pub fn usage_for(&self, agent_id: &str) -> AgentUsageSummary {
        let mut summary = AgentUsageSummary::default();
        for event in self.events_for(agent_id) {
            summary.total_events += 1;
            if event.used {
                summary.used_count += 1;
                match &summary.last_used_at {
                    None => summary.last_used_at = Some(event.recorded_at.clone()),
                    Some(prev) if &event.recorded_at > prev => {
                        summary.last_used_at = Some(event.recorded_at.clone());
                    }
                    _ => {}
                }
            }
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_knowledge::runtime::KnowledgeVersions;

    fn versions() -> KnowledgeVersions {
        KnowledgeVersions {
            embedder_model: "hash-v1".to_string(),
            rules_version: None,
            infer_version: None,
        }
    }

    fn event(agent: &str, fingerprint: &str, used: bool, recorded_at: &str) -> RetrievalEvent {
        RetrievalEvent {
            agent_id: agent.to_string(),
            query_fingerprint: fingerprint.to_string(),
            hit_ids: vec!["doc:1".to_string(), "doc:2".to_string()],
            versions: versions(),
            used,
            recorded_at: recorded_at.to_string(),
        }
    }

    #[test]
    fn append_and_len_round_trip() {
        let mut store = RetrievalFeedbackStore::new();
        assert!(store.is_empty());
        store
            .append(event("agent-a", "fp1", true, "2026-06-01T00:00:00Z"))
            .expect("append");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn validate_rejects_empty_agent_id() {
        let mut store = RetrievalFeedbackStore::new();
        let err = store
            .append(event("", "fp1", false, "2026-06-01T00:00:00Z"))
            .expect_err("empty agent_id must be rejected");
        assert!(err.contains("agent_id"), "error: {err}");
    }

    #[test]
    fn validate_rejects_empty_fingerprint() {
        let mut store = RetrievalFeedbackStore::new();
        let err = store
            .append(event("agent-a", "", false, "2026-06-01T00:00:00Z"))
            .expect_err("empty fingerprint must be rejected");
        assert!(err.contains("query_fingerprint"), "error: {err}");
    }

    #[test]
    fn validate_rejects_empty_embedder_model() {
        let mut store = RetrievalFeedbackStore::new();
        let bad_versions = KnowledgeVersions {
            embedder_model: String::new(),
            rules_version: None,
            infer_version: None,
        };
        let ev = RetrievalEvent {
            versions: bad_versions,
            ..event("agent-a", "fp1", false, "2026-06-01T00:00:00Z")
        };
        let err = store.append(ev).expect_err("empty embedder_model rejected");
        assert!(err.contains("embedder_model"), "error: {err}");
    }

    #[test]
    fn per_agent_cap_evicts_oldest_and_warns() {
        // cap = 2: after 3 inserts the first (oldest) is gone.
        let mut store = RetrievalFeedbackStore::new().with_per_agent_cap(2);
        store
            .append(event("agent-a", "fp1", false, "2026-06-01T00:00:00Z"))
            .expect("append 1");
        store
            .append(event("agent-a", "fp2", false, "2026-06-01T00:01:00Z"))
            .expect("append 2");
        store
            .append(event("agent-a", "fp3", false, "2026-06-01T00:02:00Z"))
            .expect("append 3");
        // cap=2: fp1 is evicted; fp2 and fp3 remain.
        let fingerprints: Vec<_> = store
            .events_for("agent-a")
            .map(|e| e.query_fingerprint.as_str())
            .collect();
        assert_eq!(fingerprints, vec!["fp2", "fp3"], "oldest fp1 evicted");
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn global_cap_evicts_oldest_across_agents() {
        // global_cap=2: after 3 inserts (different agents) the first is gone.
        let mut store = RetrievalFeedbackStore::new().with_global_cap(2);
        store
            .append(event("agent-a", "fp1", false, "2026-06-01T00:00:00Z"))
            .expect("append 1");
        store
            .append(event("agent-b", "fp2", false, "2026-06-01T00:01:00Z"))
            .expect("append 2");
        store
            .append(event("agent-c", "fp3", false, "2026-06-01T00:02:00Z"))
            .expect("append 3");
        // fp1 (agent-a) is the oldest and is evicted globally.
        assert_eq!(store.len(), 2);
        assert_eq!(store.events_for("agent-a").count(), 0);
    }

    #[test]
    fn usage_for_summarises_correctly() {
        let mut store = RetrievalFeedbackStore::new();
        store
            .append(event("agent-a", "fp1", true, "2026-06-01T00:00:00Z"))
            .expect("a");
        store
            .append(event("agent-a", "fp2", false, "2026-06-01T00:01:00Z"))
            .expect("b");
        store
            .append(event("agent-a", "fp3", true, "2026-06-01T00:02:00Z"))
            .expect("c");

        let summary = store.usage_for("agent-a");
        assert_eq!(summary.total_events, 3);
        assert_eq!(summary.used_count, 2);
        assert_eq!(
            summary.last_used_at.as_deref(),
            Some("2026-06-01T00:02:00Z")
        );
    }

    #[test]
    fn usage_for_unknown_agent_returns_zero_summary() {
        let store = RetrievalFeedbackStore::new();
        let summary = store.usage_for("nobody");
        assert_eq!(summary.total_events, 0);
        assert_eq!(summary.used_count, 0);
        assert!(summary.last_used_at.is_none());
    }

    #[test]
    fn hit_ids_truncated_at_max() {
        let mut store = RetrievalFeedbackStore::new();
        let hit_ids: Vec<String> = (0..=MAX_HIT_IDS + 10).map(|i| format!("doc:{i}")).collect();
        let ev = RetrievalEvent {
            hit_ids,
            ..event("agent-a", "fp1", true, "2026-06-01T00:00:00Z")
        };
        store.append(ev).expect("append truncated hits");
        let stored = store.events_for("agent-a").next().expect("event stored");
        assert_eq!(stored.hit_ids.len(), MAX_HIT_IDS);
    }

    #[test]
    fn validate_restored_catches_invalid_event() {
        // Manually construct a store with an invalid event (bypassing append).
        let mut store = RetrievalFeedbackStore::new();
        store.events.push_back(RetrievalEvent {
            agent_id: String::new(), // invalid
            query_fingerprint: "fp1".to_string(),
            hit_ids: Vec::new(),
            versions: versions(),
            used: false,
            recorded_at: "2026-06-01T00:00:00Z".to_string(),
        });
        let err = store
            .validate_restored()
            .expect_err("invalid event detected");
        assert!(err.contains("agent_id"), "error: {err}");
    }

    #[test]
    fn serde_round_trip() {
        let mut store = RetrievalFeedbackStore::new();
        store
            .append(event("agent-a", "fp1", true, "2026-06-01T00:00:00Z"))
            .expect("append");
        let json = serde_json::to_string(&store).expect("serialize");
        let restored: RetrievalFeedbackStore = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(store.len(), restored.len());
        let orig = store.events().next().expect("orig event");
        let back = restored.events().next().expect("restored event");
        assert_eq!(orig.agent_id, back.agent_id);
        assert_eq!(orig.query_fingerprint, back.query_fingerprint);
        assert_eq!(orig.used, back.used);
    }
}
