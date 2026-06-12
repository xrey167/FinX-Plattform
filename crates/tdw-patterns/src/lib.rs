#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]

//! Deterministic frequent-motif mining over the temporal knowledge graph (knowledge-system K-R4).
//!
//! # What a "motif" is in v1
//!
//! A **motif** in this version is a **set of edge-relationship-type labels**
//! (e.g. `{"listed_on", "supplier_of"}`), identified by up to
//! [`MiningLimits::max_motif_edges`] labels (default 3).
//!
//! The mining pass counts how many distinct **source entities** (`from` nodes)
//! participate in **all** edge-relationship types in the set simultaneously.
//! That count is the motif's *support*. A motif survives the pass only when
//! `support ≥ min_support`.
//!
//! Example:
//!
//! ```text
//! Graph edges:
//!   instrument:AAPL --listed_on--> venue:NASDAQ
//!   instrument:MSFT --listed_on--> venue:NASDAQ
//!   instrument:AAPL --supplier_of--> instrument:MSFT
//!   instrument:MSFT --supplier_of--> instrument:GOOG
//!
//! Motif {"listed_on", "supplier_of"}:
//!   from-entities in listed_on:  {AAPL, MSFT}
//!   from-entities in supplier_of: {AAPL, MSFT}
//!   intersection: {AAPL, MSFT} → support = 2
//! ```
//!
//! **Limitation (documented evolution):** v1 matches only on `from`-entity
//! overlap — it does not verify that the matched edges form a topologically
//! connected subgraph (e.g. it does not check that the `to` side of
//! `listed_on` is the same node as some other edge's endpoint). Full
//! connected-subgraph motif semantics are planned for a future mining version
//! and will be introduced as a non-breaking opt-in (`MiningLimits::connected`).
//!
//! # Motif encoding
//!
//! A motif is identified by a **canonical string**: the edge-label sequence
//! sorted lexicographically and joined with `"::"`. Sorting makes the identity
//! stable regardless of traversal order:
//!
//! ```text
//! motif labels: ["listed_on", "supplier_of"]
//! canonical:    "listed_on::supplier_of"
//! ```
//!
//! Two mining runs over the same graph always produce the same canonical string
//! for the same label set — the determinism gate test verifies this.
//!
//! # Idempotency
//!
//! Re-mining updates the support count on an existing `Pattern` node and
//! refreshes its instance provenance edges; it does **not** duplicate nodes.
//! Pattern node identity is `pattern:<canonical>` (stable across runs).
//!
//! # Inference posture (`tdw-infer` interaction)
//!
//! `pattern_instance_of` edges written by the miner are **excluded** from the
//! miner's own edge collection (see [`mining::PatternEngine`]) so re-mining
//! cannot discover its own provenance edges as new motifs.
//!
//! In `tdw-infer`, rules fire only when their `when` clause names the specific
//! relationship type. Because no shipped rule names `pattern_instance_of` as a
//! `when` rel, these edges cannot accidentally trigger derivation rules.
//! If a future rule is authored that consumes `pattern_instance_of`, it must be
//! reviewed explicitly for derivation-from-derivation safety — this is a design
//! decision, not an accident.
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
