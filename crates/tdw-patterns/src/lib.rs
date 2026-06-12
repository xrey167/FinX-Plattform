#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Deterministic frequent-motif mining over the temporal knowledge graph (knowledge-system K-R4).
//!
//! A **motif** is a small labeled subgraph shape defined by up to
//! [`MiningLimits::max_motif_edges`] edge-relationship types (default 3). The
//! mining pass counts how many distinct entity triples match each shape
//! (support counting) within a temporal window, persists matching shapes as
//! [`EntityKind::Pattern`] nodes with `described_by`-style provenance edges to
//! their instances, and returns a [`PatternIndex`] the caller owns and persists.
//!
//! # Motif encoding
//!
//! A motif is identified by a **canonical string**: the edge-label sequence
//! sorted lexicographically and joined with `"::"`. Sorting makes the identity
//! stable regardless of traversal order:
//!
//! ```text
//! motif for (a --listed_on--> v, a --supplier_of--> b)
//!   → sorted labels: ["listed_on", "supplier_of"]
//!   → canonical: "listed_on::supplier_of"
//! ```
//!
//! Two mining runs over the same graph always produce the same canonical string
//! for the same shape — the determinism gate test verifies this.
//!
//! # Idempotency
//!
//! Re-mining updates the support count on an existing `Pattern` node and
//! refreshes its instance provenance edges; it does **not** duplicate nodes.
//! Pattern node identity is `pattern:<canonical>` (stable across runs).
//!
//! # Hard bounds (B7 posture)
//!
//! Every bound is a hard error ([`PatternError`]), never silent truncation:
//!
//! | Bound | Field | Default |
//! |---|---|---|
//! | Candidate edge-pair cap | `max_candidates` | 8 000 |
//! | Instance-scan budget | `max_instance_scan` | 20 000 |
//! | Runtime budget (iterations) | `max_iterations` | 50 000 |
//! | Max provenance edges per pattern | `max_provenance_edges` | 64 |
//!
//! # Persistence
//!
//! [`PatternIndex`] is caller-owned; nothing in this crate touches the
//! filesystem. Round-trip via [`PatternIndex::to_json`] /
//! [`PatternIndex::from_json`].
//!
//! # Status note (enabled = false default)
//!
//! Pattern mining is disabled by default (`[knowledge.patterns] enabled = false`).
//! The operator must flip `enabled = true` in the daemon TOML before the cron
//! trigger fires. A loud status note is emitted at every tick when disabled.
//! This is an explicit, honest default — mining is new and potentially heavy.

pub mod index;
pub mod mining;

pub use index::{PatternIndex, PatternRecord};
pub use mining::{MiningLimits, MiningReport, PatternEngine, PatternError};
