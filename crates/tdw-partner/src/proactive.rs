//! The proactive layer — one brief + nudge stream (the partner design §2,
//! partner-system W3).
//!
//! The proactive *primitives* already exist as pure compute (`tdw-alerts` /
//! `tdw-alert-evaluator` fired alerts, and the knowledge signal verbs
//! `tdw.kg.{digest,thesis_health,questions,diff}`) but nothing *unifies* them
//! into one "what changed / what I noticed / what needs you" stream. This module
//! is that unifier:
//!
//! - [`Nudge`] — one item in the stream (the partner design §2.2).
//! - [`BriefInputs`] — the unified signal sources, already collected by the
//!   adapter (fired alerts + the four KG signals). [`build_brief`] is **pure
//!   given its inputs** (the partner design §2.2), so it is deterministic and
//!   golden-testable; the adapter does the I/O of gathering the signals.
//! - [`build_brief`] — fan the signals into one list ranked by
//!   severity × recency (the partner design §2.2). Determinism is the W3.1 gate.
//! - [`rerank_with_dismissals`] — a [`Dismissal`] is a feedback signal; this is
//!   the W3.5 closure that lets a repeatedly-dismissed nudge kind sink in the
//!   ranking. The same dismissal feeds `tdw.kg.feedback` → `self_tune`/`lessons`
//!   through the adapter (B9-gated, the partner design §2.3) — this function is
//!   only the local re-rank the gated parameter would drive.
//!
//! This is the anti-over-engineering line (the partner design §9 DROP "a new
//! scheduler"): the scheduling itself lives in [`crate::scheduler`] and reuses
//! `tdw-cron`; this module is the pure assembler the schedule fans into.

use serde::{Deserialize, Serialize};

/// How urgent a [`Nudge`] is — the primary ranking key (highest first).
///
/// Ordered so that `Critical > High > Medium > Low > Info`: the derived `Ord`
/// follows declaration order, and [`build_brief`] reverses it so the most severe
/// nudge ranks first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    /// Background context; never escalates.
    Info,
    /// Worth a glance.
    Low,
    /// Should be reviewed today.
    Medium,
    /// Needs attention soon.
    High,
    /// Needs attention now (a fired alert, a broken thesis).
    Critical,
}

impl Severity {
    /// A stable numeric rank (higher = more severe) for the score.
    #[must_use]
    pub const fn rank(self) -> i64 {
        match self {
            Self::Info => 0,
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::Critical => 4,
        }
    }
}

/// What produced a [`Nudge`] (the partner design §2.2 `NudgeKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NudgeKind {
    /// A `tdw-alerts` price alert fired (`tdw-alert-evaluator`).
    AlertFired,
    /// A thesis's health degraded (`tdw.kg.thesis_health`).
    ThesisHealth,
    /// An open question needs an answer (`tdw.kg.questions`).
    OpenQuestion,
    /// A contradiction / change was detected (`tdw.kg.diff`).
    Contradiction,
    /// Knowledge is going stale (`tdw.kg.digest`).
    Staleness,
    /// An autonomous action was taken (the audit trail, the partner design §4).
    ActionTaken,
}

impl NudgeKind {
    /// The stable wire token for this kind (used in the dismissal key and the
    /// serialized surface).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AlertFired => "alert_fired",
            Self::ThesisHealth => "thesis_health",
            Self::OpenQuestion => "open_question",
            Self::Contradiction => "contradiction",
            Self::Staleness => "staleness",
            Self::ActionTaken => "action_taken",
        }
    }
}

/// A record that a nudge (kind) was dismissed — the feedback signal (the partner
/// design §2.3).
///
/// A dismissal both lowers a single nudge's local rank here AND is routed by the
/// adapter through `tdw.kg.feedback` into `self_tune`/`lessons` (B9-gated). The
/// `count` is how many times this kind was dismissed in the trust window, which
/// is what drives the suppression curve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dismissal {
    /// The nudge kind that was dismissed.
    pub kind: NudgeKind,
    /// How many times this kind was dismissed in the recent window.
    pub count: u32,
}

/// One item in the proactive stream (the partner design §2.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nudge {
    /// Stable id (deterministic, derived from the source so re-runs dedupe).
    pub id: String,
    /// What produced it.
    pub kind: NudgeKind,
    /// How urgent it is — the primary ranking key.
    pub severity: Severity,
    /// The one-line summary a surface renders.
    pub headline: String,
    /// The route ids / KG node ids / alert id that produced it.
    pub provenance: crate::Provenance,
    /// When it was created (`YYYY-MM-DD` or RFC 3339), the recency tie-breaker.
    pub created_at: String,
    /// Whether it was dismissed (set by the surface; feeds learning §2.3).
    #[serde(default)]
    pub dismissed: bool,
}

/// A fired-alert signal, reduced to what a [`Nudge`] needs (the partner design
/// §2.1: the `tdw-alerts`/`tdw-alert-evaluator` output).
///
/// The adapter maps a `tdw_alerts::PriceAlert` that just fired into this; the
/// pure assembler never touches the alert store, keeping `tdw-partner` a leaf.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiredAlert {
    /// The alert id (the nudge provenance + dedup key).
    pub id: String,
    /// The symbol the alert watched.
    pub symbol: String,
    /// The headline the surface renders.
    pub headline: String,
    /// When it fired (`YYYY-MM-DD` / RFC 3339).
    pub fired_at: String,
}

/// A knowledge signal, reduced to what a [`Nudge`] needs.
///
/// The adapter maps one item out of a `tdw.kg.{thesis_health,questions,diff,
/// digest}` result into this. The `severity` is the signal's own urgency (e.g.
/// a regressed thesis is `Critical`, a staleness item is `Low`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeSignal {
    /// A stable id for the signal (a KG node id; the dedup key).
    pub id: String,
    /// Which knowledge signal kind it is.
    pub kind: NudgeKind,
    /// The signal's own urgency.
    pub severity: Severity,
    /// The headline the surface renders.
    pub headline: String,
    /// The KG node ids backing it.
    #[serde(default)]
    pub kg_nodes: Vec<String>,
    /// When it was observed (`YYYY-MM-DD` / RFC 3339).
    pub as_of: String,
}

/// The unified signal sources for one brief (the partner design §2.2
/// `BriefInputs`).
///
/// Already collected by the adapter (the I/O lane); [`build_brief`] is pure over
/// this. Wiring: `alerts` ← `tdw-alerts`/`tdw-alert-evaluator`; `signals` ←
/// `tdw.kg.{thesis_health,questions,diff,digest}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BriefInputs {
    /// Fired price alerts (always `Critical`, the most urgent class).
    #[serde(default)]
    pub alerts: Vec<FiredAlert>,
    /// Knowledge signals (thesis health / open questions / contradictions /
    /// staleness), each carrying its own severity.
    #[serde(default)]
    pub signals: Vec<KnowledgeSignal>,
}

/// Assemble the brief: the union of the unified signal sources, ranked by
/// severity × recency (the partner design §2.2). **Pure given its inputs** — the
/// W3.1 determinism gate.
///
/// Ranking is total and stable: primary key severity (most severe first),
/// then recency (newest `created_at` first), then the id (lexicographic) as the
/// final tie-break so the order is fully determined by the inputs. A fired
/// alert is always `Critical` so it leads; the KG signals fall in by their own
/// severity. The `principal` scopes the brief (provenance / namespace) but does
/// not change the ranking, so the function stays a pure fold over its inputs.
#[must_use]
pub fn build_brief(inputs: &BriefInputs, principal: &crate::Principal) -> Vec<Nudge> {
    let mut nudges: Vec<Nudge> = Vec::with_capacity(inputs.alerts.len() + inputs.signals.len());

    for alert in &inputs.alerts {
        nudges.push(Nudge {
            id: format!("nudge:alert:{}", alert.id),
            kind: NudgeKind::AlertFired,
            severity: Severity::Critical,
            headline: alert.headline.clone(),
            provenance: crate::Provenance {
                routes: vec![format!("alert:{}", alert.id)],
                kg_nodes: Vec::new(),
                as_of: Some(alert.fired_at.clone()),
            },
            created_at: alert.fired_at.clone(),
            dismissed: false,
        });
    }

    for signal in &inputs.signals {
        nudges.push(Nudge {
            id: format!("nudge:{}:{}", signal.kind.as_str(), signal.id),
            kind: signal.kind,
            severity: signal.severity,
            headline: signal.headline.clone(),
            provenance: crate::Provenance {
                routes: Vec::new(),
                kg_nodes: signal.kg_nodes.clone(),
                as_of: Some(signal.as_of.clone()),
            },
            created_at: signal.as_of.clone(),
            dismissed: false,
        });
    }

    // Dedup duplicate nudges by id BEFORE sorting (Gemini #440 fix): the same
    // source signal can arrive twice (e.g. an alert echoed by two feeds), and
    // two arrivals of one id may carry DIFFERING `created_at` / severity. The
    // previous `dedup_by` ran AFTER the sort and only removed *consecutive*
    // equal-id entries — but differing timestamps sort the duplicates apart, so
    // a duplicate survived. A set-based retain collapses every duplicate id
    // regardless of order, keeping the first occurrence, then the sort imposes
    // the final ranking.
    let mut seen = std::collections::BTreeSet::new();
    nudges.retain(|nudge| seen.insert(nudge.id.clone()));
    sort_nudges(&mut nudges, &[]);
    let _ = principal; // scopes provenance/namespace, not the ranking.
    nudges
}

/// Re-rank an assembled brief given dismissal feedback (the partner design §2.3,
/// W3.5).
///
/// Repeatedly dismissing a nudge *kind* lowers its effective rank: the kind's
/// dismissal `count` subtracts from its score, so a noisy kind sinks below
/// fresher, un-dismissed nudges of the same nominal severity. This mirrors the
/// gated tuned parameter (the real suppression is a B9-gated `self_tune`
/// output); here it is the deterministic local effect that parameter drives, so
/// the surface re-ranks consistently. Pure and stable like [`build_brief`].
#[must_use]
pub fn rerank_with_dismissals(mut nudges: Vec<Nudge>, dismissals: &[Dismissal]) -> Vec<Nudge> {
    sort_nudges(&mut nudges, dismissals);
    nudges
}

/// The dismissal penalty for a nudge kind: the recorded dismissal `count` for
/// that kind (0 when never dismissed). Bounded so it can lower but never invert
/// the severity ordering beyond one full severity step per few dismissals.
///
/// The count is capped at [`MAX_DISMISSAL_PENALTY`] (Gemini #439 fix): an
/// unbounded `count` could grow past the per-class severity headroom (8) and
/// invert the severity ordering, sinking a `Critical` nudge below a `Low` one.
/// Capping at 7 keeps the penalty within a single severity step, so a heavily
/// dismissed kind sinks within its class but never crosses a class boundary.
fn dismissal_penalty(kind: NudgeKind, dismissals: &[Dismissal]) -> i64 {
    dismissals
        .iter()
        .find(|d| d.kind == kind)
        .map_or(0, |d| i64::from(d.count).min(MAX_DISMISSAL_PENALTY))
}

/// The cap on a dismissal penalty (Gemini #439 fix).
///
/// One less than the per-class severity scale (8) so the penalty can reorder
/// within a severity class but can never push a nudge across a full class
/// boundary, regardless of how many times its kind was dismissed.
const MAX_DISMISSAL_PENALTY: i64 = 7;

/// The total, stable comparison score for a nudge.
///
/// Severity dominates (scaled so a dismissal penalty cannot flip two nudges
/// across a full severity class for small counts), then the dismissal penalty
/// nudges within a class, then recency, then id. Returned as a tuple so the sort
/// is total and fully determined by the inputs.
fn nudge_sort_key<'a>(nudge: &'a Nudge, dismissals: &[Dismissal]) -> (i64, &'a str, &'a str) {
    // Severity scaled by 8 leaves headroom for the dismissal penalty to reorder
    // within a class (penalty is small) without crossing a class boundary.
    let score = nudge.severity.rank() * 8 - dismissal_penalty(nudge.kind, dismissals);
    (score, nudge.created_at.as_str(), nudge.id.as_str())
}

/// Sort nudges in place by the stable key: score desc, then recency desc, then
/// id asc. Extracted so [`build_brief`] and [`rerank_with_dismissals`] share one
/// ordering definition.
fn sort_nudges(nudges: &mut [Nudge], dismissals: &[Dismissal]) {
    nudges.sort_by(|a, b| {
        let (sa, ca, ia) = nudge_sort_key(a, dismissals);
        let (sb, cb, ib) = nudge_sort_key(b, dismissals);
        // score desc, created_at desc (newer first), id asc.
        sb.cmp(&sa).then(cb.cmp(ca)).then(ia.cmp(ib))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal() -> crate::Principal {
        crate::Principal::new("sess", "agent:partner")
    }

    fn fixed_inputs() -> BriefInputs {
        BriefInputs {
            alerts: vec![FiredAlert {
                id: "a-1".to_string(),
                symbol: "AAPL".to_string(),
                headline: "AAPL crossed $200".to_string(),
                fired_at: "2026-06-14".to_string(),
            }],
            signals: vec![
                KnowledgeSignal {
                    id: "thesis-7".to_string(),
                    kind: NudgeKind::ThesisHealth,
                    severity: Severity::High,
                    headline: "Thesis 'AAPL services moat' weakening".to_string(),
                    kg_nodes: vec!["finding:thesis-7".to_string()],
                    as_of: "2026-06-13".to_string(),
                },
                KnowledgeSignal {
                    id: "q-3".to_string(),
                    kind: NudgeKind::OpenQuestion,
                    severity: Severity::Medium,
                    headline: "Open question: MSFT cloud margin trend?".to_string(),
                    kg_nodes: vec!["question:q-3".to_string()],
                    as_of: "2026-06-14".to_string(),
                },
                KnowledgeSignal {
                    id: "stale-9".to_string(),
                    kind: NudgeKind::Staleness,
                    severity: Severity::Low,
                    headline: "5 findings going stale".to_string(),
                    kg_nodes: vec!["finding:stale-9".to_string()],
                    as_of: "2026-06-12".to_string(),
                },
            ],
        }
    }

    // ── W3.1: ranking determinism ───────────────────────────────────────────

    #[test]
    fn build_brief_ranks_by_severity_then_recency() {
        let brief = build_brief(&fixed_inputs(), &principal());
        let order: Vec<&str> = brief.iter().map(|n| n.headline.as_str()).collect();
        // Critical alert first, then High thesis, Medium question, Low staleness.
        assert_eq!(
            order,
            vec![
                "AAPL crossed $200",
                "Thesis 'AAPL services moat' weakening",
                "Open question: MSFT cloud margin trend?",
                "5 findings going stale",
            ],
            "severity-major, recency-minor ranking"
        );
    }

    #[test]
    fn build_brief_is_deterministic_across_input_order() {
        // The golden brief from fixed inputs is identical regardless of the order
        // the signals arrive in — the W3.1/W3.2 determinism gate.
        let mut shuffled = fixed_inputs();
        shuffled.signals.reverse();
        let a = build_brief(&fixed_inputs(), &principal());
        let b = build_brief(&shuffled, &principal());
        assert_eq!(a, b, "ranking is a pure function of the input set");
    }

    #[test]
    fn build_brief_breaks_ties_by_recency_then_id() {
        // Two same-severity signals: newer as_of wins; equal as_of → id asc.
        let inputs = BriefInputs {
            alerts: Vec::new(),
            signals: vec![
                KnowledgeSignal {
                    id: "z".to_string(),
                    kind: NudgeKind::OpenQuestion,
                    severity: Severity::Medium,
                    headline: "older".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-10".to_string(),
                },
                KnowledgeSignal {
                    id: "a".to_string(),
                    kind: NudgeKind::OpenQuestion,
                    severity: Severity::Medium,
                    headline: "newer".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-14".to_string(),
                },
            ],
        };
        let brief = build_brief(&inputs, &principal());
        assert_eq!(brief[0].headline, "newer", "newer as_of ranks first");
    }

    #[test]
    fn empty_inputs_yield_empty_brief() {
        let brief = build_brief(&BriefInputs::default(), &principal());
        assert!(brief.is_empty());
    }

    #[test]
    fn nudge_ids_are_deterministic_and_namespaced() {
        let brief = build_brief(&fixed_inputs(), &principal());
        assert_eq!(brief[0].id, "nudge:alert:a-1");
        assert!(brief.iter().any(|n| n.id == "nudge:thesis_health:thesis-7"));
    }

    // ── W3.5: dismissal feeds re-rank (B9-gated at the surface) ──────────────

    #[test]
    fn dismissing_a_kind_lowers_its_rank() {
        // Two Medium signals of different kinds at the same recency; dismissing
        // one kind heavily should sink it below the other.
        let inputs = BriefInputs {
            alerts: Vec::new(),
            signals: vec![
                KnowledgeSignal {
                    id: "noisy".to_string(),
                    kind: NudgeKind::Staleness,
                    severity: Severity::Medium,
                    // Newer, so it leads on the recency tie-break pre-dismissal.
                    headline: "noisy staleness".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-14".to_string(),
                },
                KnowledgeSignal {
                    id: "useful".to_string(),
                    kind: NudgeKind::OpenQuestion,
                    severity: Severity::Medium,
                    headline: "useful question".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-13".to_string(),
                },
            ],
        };
        let brief = build_brief(&inputs, &principal());
        // Without dismissals, equal severity → newer as_of leads (noisy first).
        assert_eq!(brief[0].headline, "noisy staleness");

        let reranked = rerank_with_dismissals(
            brief,
            &[Dismissal {
                kind: NudgeKind::Staleness,
                count: 4,
            }],
        );
        assert_eq!(
            reranked[0].headline, "useful question",
            "a repeatedly-dismissed kind sinks below an un-dismissed peer"
        );
    }

    #[test]
    fn dismissal_never_crosses_a_full_severity_class_for_small_counts() {
        // A dismissed High nudge must still outrank a fresh Low nudge: the
        // suppression re-orders within a class, it does not erase severity.
        let inputs = BriefInputs {
            alerts: Vec::new(),
            signals: vec![
                KnowledgeSignal {
                    id: "high".to_string(),
                    kind: NudgeKind::ThesisHealth,
                    severity: Severity::High,
                    headline: "high".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-14".to_string(),
                },
                KnowledgeSignal {
                    id: "low".to_string(),
                    kind: NudgeKind::Staleness,
                    severity: Severity::Low,
                    headline: "low".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-14".to_string(),
                },
            ],
        };
        let brief = build_brief(&inputs, &principal());
        let reranked = rerank_with_dismissals(
            brief,
            &[Dismissal {
                kind: NudgeKind::ThesisHealth,
                count: 3,
            }],
        );
        assert_eq!(
            reranked[0].headline, "high",
            "severity still dominates a small dismissal penalty"
        );
    }

    // ── Gemini #439: dedup duplicate-id nudges ───────────────────────────────

    #[test]
    fn build_brief_dedups_duplicate_id_nudges() {
        // The same alert echoed by two feeds yields two FiredAlerts with the same
        // id; the brief must render the nudge once, not twice.
        let inputs = BriefInputs {
            alerts: vec![
                FiredAlert {
                    id: "dup-1".to_string(),
                    symbol: "AAPL".to_string(),
                    headline: "AAPL crossed $200".to_string(),
                    fired_at: "2026-06-14".to_string(),
                },
                FiredAlert {
                    id: "dup-1".to_string(),
                    symbol: "AAPL".to_string(),
                    headline: "AAPL crossed $200".to_string(),
                    fired_at: "2026-06-14".to_string(),
                },
            ],
            signals: vec![
                KnowledgeSignal {
                    id: "q-dup".to_string(),
                    kind: NudgeKind::OpenQuestion,
                    severity: Severity::Medium,
                    headline: "duplicate question".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-13".to_string(),
                },
                KnowledgeSignal {
                    id: "q-dup".to_string(),
                    kind: NudgeKind::OpenQuestion,
                    severity: Severity::Medium,
                    headline: "duplicate question".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-13".to_string(),
                },
            ],
        };
        let brief = build_brief(&inputs, &principal());
        assert_eq!(brief.len(), 2, "two distinct ids survive: {brief:?}");
        let ids: Vec<&str> = brief.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"nudge:alert:dup-1"));
        assert!(ids.contains(&"nudge:open_question:q-dup"));
        // No id appears twice.
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), brief.len(), "every id is unique: {ids:?}");
    }

    #[test]
    fn build_brief_dedups_same_id_with_differing_timestamp_and_severity() {
        // Gemini #440: the same logical signal can arrive twice with DIFFERING
        // created_at AND severity (two feeds disagree on the metadata). These
        // map to one nudge id but do NOT sort adjacently — the old sort-then-
        // `dedup_by` left a duplicate. The set-based dedup-before-sort must
        // still collapse them to a single nudge.
        let inputs = BriefInputs {
            alerts: Vec::new(),
            signals: vec![
                KnowledgeSignal {
                    id: "thesis-x".to_string(),
                    kind: NudgeKind::ThesisHealth,
                    severity: Severity::High,
                    headline: "thesis weakening (feed A)".to_string(),
                    kg_nodes: Vec::new(),
                    // Older timestamp.
                    as_of: "2026-06-10".to_string(),
                },
                KnowledgeSignal {
                    id: "thesis-x".to_string(),
                    kind: NudgeKind::ThesisHealth,
                    // A DIFFERENT severity for the same id...
                    severity: Severity::Critical,
                    headline: "thesis weakening (feed B)".to_string(),
                    kg_nodes: Vec::new(),
                    // ...and a NEWER timestamp, so post-sort the two would land
                    // far apart and a consecutive-only dedup would miss one.
                    as_of: "2026-06-14".to_string(),
                },
                // A second distinct id so the surviving count is unambiguous.
                KnowledgeSignal {
                    id: "q-1".to_string(),
                    kind: NudgeKind::OpenQuestion,
                    severity: Severity::Medium,
                    headline: "open question".to_string(),
                    kg_nodes: Vec::new(),
                    as_of: "2026-06-12".to_string(),
                },
            ],
        };
        let brief = build_brief(&inputs, &principal());
        let thesis_count = brief
            .iter()
            .filter(|n| n.id == "nudge:thesis_health:thesis-x")
            .count();
        assert_eq!(
            thesis_count, 1,
            "the same id with differing timestamp/severity collapses to one: {brief:?}"
        );
        assert_eq!(
            brief.len(),
            2,
            "exactly two distinct ids survive: {brief:?}"
        );
        // The FIRST occurrence is the one kept (feed A), proving a stable,
        // order-defined dedup rather than a severity- or recency-driven pick.
        let kept = brief
            .iter()
            .find(|n| n.id == "nudge:thesis_health:thesis-x")
            .expect("the thesis nudge survives");
        assert_eq!(kept.headline, "thesis weakening (feed A)");
    }

    #[test]
    fn dismissal_penalty_is_capped_and_never_inverts_severity() {
        // A massively-dismissed Critical nudge must still outrank a fresh Low
        // nudge: the penalty is capped at MAX_DISMISSAL_PENALTY (< the per-class
        // scale of 8) so it can never cross a severity class (Gemini #439).
        let inputs = BriefInputs {
            alerts: vec![FiredAlert {
                id: "crit".to_string(),
                symbol: "AAPL".to_string(),
                headline: "critical alert".to_string(),
                fired_at: "2026-06-14".to_string(),
            }],
            signals: vec![KnowledgeSignal {
                id: "low".to_string(),
                kind: NudgeKind::Staleness,
                severity: Severity::Low,
                headline: "low staleness".to_string(),
                kg_nodes: Vec::new(),
                as_of: "2026-06-14".to_string(),
            }],
        };
        let brief = build_brief(&inputs, &principal());
        // An absurd dismissal count for the Critical kind — pre-cap this would
        // drive the score below the Low nudge and invert the ordering.
        let reranked = rerank_with_dismissals(
            brief,
            &[Dismissal {
                kind: NudgeKind::AlertFired,
                count: 1_000_000,
            }],
        );
        assert_eq!(
            reranked[0].headline, "critical alert",
            "a capped dismissal penalty never inverts the severity ordering"
        );
    }

    #[test]
    fn nudge_round_trips_through_serde() {
        let brief = build_brief(&fixed_inputs(), &principal());
        let json = serde_json::to_string(&brief).expect("serialize");
        let back: Vec<Nudge> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(brief, back);
    }
}
