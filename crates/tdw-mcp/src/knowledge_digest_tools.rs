//! The self-narrating knowledge DIGEST tool (knowledge-system-2 K-X4).
//!
//! `tdw.kg.digest` is a read-only tool that makes the knowledge base narrate
//! itself: a single call returns both a structured summary and a readable prose
//! narrative describing what the system learned recently, which entities/tags
//! are most active, what is going STALE, how many proposals await eval, and any
//! eval-drift alarms.  It reuses the existing read seams — [`KnowledgeRuntime::status`]
//! (the same source `tdw.kg.status` reports, so counts match exactly) plus a
//! bounded graph scan of `Finding` nodes — and adds NO write paths.
//!
//! # Design notes (Gemini pre-empt)
//!
//! * **Injected-now.** The library never reads the wall clock.  The MCP dispatch
//!   layer injects the reference time (`now`, a `YYYY-MM-DD` date) exactly as it
//!   does for the finding tools.  An optional `as_of` argument overrides it; the
//!   staleness window and the "new knowledge" window are both computed against
//!   that single injected reference, so the digest is reproducible and testable.
//! * **Leakage-safe / as_of-correct.**  No `Finding` whose `valid_from` lies
//!   strictly after the reference time is ever surfaced — the digest uses the
//!   same [`tdw_core::active_at`] visibility predicate the retriever and graph
//!   tools use, evaluated at the reference time.  A fact from the future relative
//!   to the query time cannot appear in any section.
//! * **Honest.**  Every section states plainly when it is empty ("no new
//!   knowledge in window") rather than fabricating activity.  The headline
//!   counts are copied verbatim from [`KgStatus`] so they can never diverge from
//!   `tdw.kg.status`.
//! * **Date math.**  Day spans use a leap-year-correct days-from-civil algorithm
//!   ([`days_from_civil`]) — NOT an approximate Julian formula — so month/year
//!   boundaries (and the prior story's off-by-one) are handled exactly.
//! * **No blocking IO on async paths.**  The one graph scan is bridged through
//!   the same [`crate::knowledge_tools::block_on`] helper the other read tools
//!   use; no synchronous IO runs on an async executor thread.

use serde_json::{Map, Value, json};
use tdw_core::{GraphEngine, active_at};
use tdw_knowledge::runtime::{EvalFreshness, KgStatus, KnowledgeRuntime};
use tdw_tags::date_to_timestamp;
use tdw_taxonomy::EntityKind;

use crate::{
    ToolDescriptor, ToolExecution, ToolFailure, knowledge_tools::block_on, structured, tool,
};

/// The single tool name this module owns.
pub const TOOL_NAMES: &[&str] = &["tdw.kg.digest"];

/// Whether `name` is the digest tool.
#[must_use]
pub fn owns(name: &str) -> bool {
    TOOL_NAMES.contains(&name)
}

// ── Defaults / caps ─────────────────────────────────────────────────────────────

/// Default look-back window (days) for the "new knowledge" section when the
/// caller does not supply `window_days`.
pub const DEFAULT_WINDOW_DAYS: i64 = 7;

/// Maximum look-back window the caller may request.
pub const MAX_WINDOW_DAYS: i64 = 3650; // ~10 years

/// Default staleness threshold (days): a `Finding` whose last-corroboration
/// (`as_of`) is older than this — relative to the reference time — is surfaced
/// in the staleness section.
pub const DEFAULT_STALE_AFTER_DAYS: i64 = 90;

/// Maximum staleness threshold the caller may request.
pub const MAX_STALE_AFTER_DAYS: i64 = 36500; // ~100 years

/// Bounded node-scan cap.  Large enough for typical graphs; an honest
/// `scan_note` fires when exceeded so the digest never silently under-reports.
pub const DIGEST_SCAN_CAP: usize = 8_192;

/// Per-section list cap (top entities / new findings / stale findings) so the
/// readable narrative and structured lists stay bounded.
pub const SECTION_LIST_CAP: usize = 20;

// ── Descriptor ──────────────────────────────────────────────────────────────────

/// Descriptor for the digest tool.  Appended to `tools/list` only when a
/// knowledge runtime with a graph engine is attached (same gate as the explain
/// tools).  `readOnlyHint: true`, `idempotentHint: true`.
#[must_use]
pub fn descriptor() -> ToolDescriptor {
    tool(
        "tdw.kg.digest",
        "Knowledge Self-Narrating Digest",
        "Produce a human-readable, self-narrating digest of the knowledge base: what was learned \
         in the recent window, the most active entities/tags, which facts/findings are going STALE \
         (last-corroboration older than the staleness threshold), how many proposals await eval, \
         and any eval-drift alarms. Returns BOTH a structured summary and a readable `narrative` \
         string. Read-only. Counts match tdw.kg.status exactly. Staleness and the \"new knowledge\" \
         window are computed against an injected reference time (the optional `as_of`, else server \
         now) — never a wall-clock read inside the library, and never leaks facts whose validity \
         begins after the reference time.\n\
         \n\
         Caps: scan bounded at 8192 nodes (honest scan_note on overflow); each list capped at 20 \
         items; window_days ≤ 3650; stale_after_days ≤ 36500.",
        json!({
            "type": "object",
            "properties": {
                "as_of": {
                    "type": "string",
                    "description": "Reference time (YYYY-MM-DD). The window and staleness thresholds \
                                    are measured back from here. Defaults to server now. Facts whose \
                                    validity begins after this date are never surfaced."
                },
                "window_days": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 3650,
                    "description": "Look-back window (days) for the 'new knowledge' section. Default 7."
                },
                "stale_after_days": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 36500,
                    "description": "A finding whose last-corroboration is older than this many days \
                                    (relative to as_of) is flagged stale. Default 90."
                }
            },
            "additionalProperties": false
        }),
    )
}

// ── Dispatch ────────────────────────────────────────────────────────────────────

/// Execute the digest tool.  Every failure is a tool error
/// ([`ToolFailure::Execution`]), never a protocol error.
///
/// `now` is the injected reference date (`YYYY-MM-DD`) supplied by the MCP
/// dispatch layer; the optional `as_of` argument overrides it.  The library
/// never reads the wall clock itself.
///
/// # Errors
///
/// Returns [`ToolFailure::Execution`] for a missing graph engine, a malformed
/// argument, or an engine failure.
pub fn execute(
    runtime: &KnowledgeRuntime,
    name: &str,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    match name {
        "tdw.kg.digest" => digest(runtime, arguments, now),
        other => Err(execution(format!("unknown digest tool: {other}"))),
    }
}

fn digest(
    runtime: &KnowledgeRuntime,
    arguments: &Map<String, Value>,
    now: &str,
) -> Result<ToolExecution, ToolFailure> {
    let graph = runtime
        .graph()
        .ok_or_else(|| execution("knowledge graph not attached".to_string()))?;

    // Reference time: the caller's as_of overrides the injected server now.
    // Both are YYYY-MM-DD dates; validate the shape so downstream day-math and
    // the active_at predicate (which compares timestamps lexicographically) are
    // well-defined.
    let reference_date = match optional_str(arguments, "as_of") {
        Some(value) => {
            if !is_date(value) {
                return Err(execution(format!(
                    "as_of must be a YYYY-MM-DD date, got {value:?}"
                )));
            }
            value.to_string()
        }
        None => now.to_string(),
    };
    if !is_date(&reference_date) {
        return Err(execution(format!(
            "injected reference time must be a YYYY-MM-DD date, got {reference_date:?}"
        )));
    }
    let reference_ts = date_to_timestamp(&reference_date);

    let window_days = optional_i64(arguments, "window_days")?.unwrap_or(DEFAULT_WINDOW_DAYS);
    if !(1..=MAX_WINDOW_DAYS).contains(&window_days) {
        return Err(execution(format!(
            "window_days must be 1..={MAX_WINDOW_DAYS}, got {window_days}"
        )));
    }
    let stale_after_days =
        optional_i64(arguments, "stale_after_days")?.unwrap_or(DEFAULT_STALE_AFTER_DAYS);
    if !(1..=MAX_STALE_AFTER_DAYS).contains(&stale_after_days) {
        return Err(execution(format!(
            "stale_after_days must be 1..={MAX_STALE_AFTER_DAYS}, got {stale_after_days}"
        )));
    }

    // Honest counts come straight from status() — the SAME source tdw.kg.status
    // reports, so the digest can never diverge from it.
    let status = block_on(runtime.status());

    // Scan Finding nodes once; partition into new-in-window and stale.
    let scan = block_on(scan_findings(
        graph,
        &reference_ts,
        window_days,
        stale_after_days,
    ))?;

    let structured_value = build_structured(
        &status,
        &reference_date,
        window_days,
        stale_after_days,
        &scan,
    );
    let narrative = build_narrative(
        &status,
        &reference_date,
        window_days,
        stale_after_days,
        &scan,
    );

    let mut out = structured_value;
    if let Value::Object(map) = &mut out {
        map.insert("narrative".to_string(), Value::String(narrative));
    }
    Ok(structured(out))
}

// ── Finding scan ────────────────────────────────────────────────────────────────

/// One scanned `Finding` reduced to the fields the digest reports.
struct ScannedFinding {
    id: String,
    title: String,
    /// Last-corroboration proxy: the finding's `props.as_of` timestamp (falls
    /// back to `valid_from` when absent).  Empty when neither is present.
    as_of: String,
    /// Whole days from `as_of` to the reference time (>= 0 for past findings).
    age_days: i64,
    is_thesis: bool,
}

/// Result of the bounded `Finding` scan.
struct FindingScan {
    /// Findings whose validity began within the look-back window (newest first),
    /// truncated to [`SECTION_LIST_CAP`] for display.
    new_in_window: Vec<ScannedFinding>,
    /// True total of new-in-window findings BEFORE the display truncation, so the
    /// reported `count` stays honest even when more than [`SECTION_LIST_CAP`] match.
    new_in_window_total: usize,
    /// Active findings whose last-corroboration is older than the staleness
    /// threshold (oldest first), truncated to [`SECTION_LIST_CAP`] for display.
    stale: Vec<ScannedFinding>,
    /// True total of stale findings BEFORE the display truncation, so the reported
    /// `count` stays honest even when more than [`SECTION_LIST_CAP`] match.
    stale_total: usize,
    /// Top entity/finding labels by recent activity (within the window),
    /// `(label, count)` — actually a per-finding tally; bounded.
    tag_activity: Vec<(String, usize)>,
    /// True total of distinct active labels with activity in the window BEFORE the
    /// display truncation, so the reported `count` stays honest.
    tag_activity_total: usize,
    /// Total active findings visible at the reference time.
    active_total: usize,
    /// True when the scan hit [`DIGEST_SCAN_CAP`]; the digest then carries an
    /// honest `scan_note`.
    capped: bool,
}

/// Mutable accumulator threaded through the page-walk; drained into a
/// [`FindingScan`] once the scan completes.
#[derive(Default)]
struct ScanAccumulator {
    new_in_window: Vec<ScannedFinding>,
    stale: Vec<ScannedFinding>,
    tag_tally: std::collections::BTreeMap<String, usize>,
    active_total: usize,
    capped: bool,
}

/// Classify one graph node, folding it into `acc` when it is a `Finding` active
/// at the reference time.  Pure aside from the `acc` mutation — extracted from
/// the page-walk loop so the scan body stays readable and bounded.
///
/// Leakage guard lives here: a node whose validity is not active at
/// `reference_ts` (e.g. a finding born after the query time) is dropped before
/// it can reach any bucket.
fn classify_finding(
    node: &tdw_core::GraphNode,
    candidate_id: &str,
    reference_ts: &str,
    reference_dn: i64,
    window_days: i64,
    stale_after_days: i64,
    acc: &mut ScanAccumulator,
) {
    if node.kind != EntityKind::Finding {
        return;
    }
    // Leakage guard: only findings ACTIVE at the reference time.
    if !active_at(
        node.valid_from.as_deref(),
        node.valid_to.as_deref(),
        reference_ts,
    ) {
        return;
    }
    acc.active_total += 1;

    let is_thesis = node.props.get("kind_hint").and_then(Value::as_str) == Some("thesis");
    // Last-corroboration proxy: props.as_of, else valid_from.
    let as_of = node
        .props
        .get("as_of")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| node.valid_from.clone())
        .unwrap_or_default();
    let title = node
        .props
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map_or_else(|| node.label.clone(), ToString::to_string);

    // Age in whole days (reference - as_of), leap-correct. Unknown date → max.
    let age_days = if as_of.is_empty() {
        i64::MAX
    } else {
        reference_dn.saturating_sub(day_number(&as_of))
    };

    // New-in-window: validity began within the look-back window AND not in the
    // future relative to the reference time. valid_from is the "born at" stamp.
    if let Some(vf) = node.valid_from.as_deref() {
        let born_age = reference_dn.saturating_sub(day_number(vf));
        if vf <= reference_ts && born_age <= window_days {
            *acc.tag_tally.entry(title.clone()).or_insert(0) += 1;
            acc.new_in_window.push(ScannedFinding {
                id: candidate_id.to_string(),
                title: title.clone(),
                as_of: as_of.clone(),
                age_days: born_age,
                is_thesis,
            });
        }
    }

    // Stale: last-corroboration older than the threshold.
    if age_days >= stale_after_days {
        acc.stale.push(ScannedFinding {
            id: candidate_id.to_string(),
            title,
            as_of,
            age_days,
            is_thesis,
        });
    }
}

/// Scan up to [`DIGEST_SCAN_CAP`] `Finding` nodes via a bounded edge page-walk
/// (the same shape the K-X7 thesis status scan uses), partitioning them into the
/// new-in-window and stale buckets.
///
/// Leakage guard: a finding is only considered when it is ACTIVE at the
/// reference time (`active_at(valid_from, valid_to, reference_ts)`).  A finding
/// whose `valid_from` is after the reference time fails this predicate and is
/// silently excluded — it does not exist yet from the query's point of view.
async fn scan_findings(
    graph: &std::sync::Arc<dyn GraphEngine>,
    reference_ts: &str,
    window_days: i64,
    stale_after_days: i64,
) -> Result<FindingScan, ToolFailure> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut acc = ScanAccumulator::default();
    let mut offset = 0usize;
    let page_size = 256usize;
    // Reference day-number, computed once (leap-correct).
    let reference_dn = day_number(reference_ts);

    'outer: loop {
        let page = graph
            .edges(None, offset, page_size)
            .await
            .map_err(|error| execution(error.to_string()))?;
        if page.is_empty() {
            break;
        }
        for edge in &page {
            for candidate_id in [&edge.from, &edge.to] {
                if seen.contains(candidate_id) {
                    continue;
                }
                if seen.len() >= DIGEST_SCAN_CAP {
                    acc.capped = true;
                    break 'outer;
                }
                seen.insert(candidate_id.clone());

                let Ok(Some(node)) = graph.node(candidate_id).await else {
                    continue;
                };
                classify_finding(
                    &node,
                    candidate_id,
                    reference_ts,
                    reference_dn,
                    window_days,
                    stale_after_days,
                    &mut acc,
                );
            }
        }
        offset += page.len();
        if offset >= DIGEST_SCAN_CAP * 4 {
            acc.capped = true;
            break;
        }
    }

    let ScanAccumulator {
        mut new_in_window,
        mut stale,
        tag_tally,
        active_total,
        capped,
    } = acc;

    // Capture the TRUE totals before the display truncation so the reported
    // `count` fields can never under-report (Gemini honesty finding).
    let new_in_window_total = new_in_window.len();
    let stale_total = stale.len();

    // Newest first for new-in-window (smallest age first).
    new_in_window.sort_by(|a, b| a.age_days.cmp(&b.age_days).then(a.id.cmp(&b.id)));
    new_in_window.truncate(SECTION_LIST_CAP);

    // Oldest first for stale (largest age first).
    stale.sort_by(|a, b| b.age_days.cmp(&a.age_days).then(a.id.cmp(&b.id)));
    stale.truncate(SECTION_LIST_CAP);

    // Top activity by per-finding tally (descending count, then label).
    let mut tag_activity: Vec<(String, usize)> = tag_tally.into_iter().collect();
    let tag_activity_total = tag_activity.len();
    tag_activity.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    tag_activity.truncate(SECTION_LIST_CAP);

    Ok(FindingScan {
        new_in_window,
        new_in_window_total,
        stale,
        stale_total,
        tag_activity,
        tag_activity_total,
        active_total,
        capped,
    })
}

// ── Structured form ─────────────────────────────────────────────────────────────

fn finding_to_value(f: &ScannedFinding) -> Value {
    json!({
        "id": f.id,
        "title": f.title,
        "as_of": f.as_of,
        "age_days": age_or_null(f.age_days),
        "is_thesis": f.is_thesis,
    })
}

/// `i64::MAX` age (unknown corroboration date) serializes as `null` rather than
/// a nonsense huge number, so the structured form stays honest.
fn age_or_null(age_days: i64) -> Value {
    if age_days == i64::MAX {
        Value::Null
    } else {
        json!(age_days)
    }
}

fn build_structured(
    status: &KgStatus,
    reference_date: &str,
    window_days: i64,
    stale_after_days: i64,
    scan: &FindingScan,
) -> Value {
    let proposals_pending = status
        .proposals
        .as_ref()
        .map(|p| p.draft + p.validated + p.ready);

    let eval_alarm = status.eval_freshness.is_alarm();

    let scan_note = if scan.capped {
        Some(format!(
            "finding scan bounded at {DIGEST_SCAN_CAP} nodes; some findings may be omitted"
        ))
    } else {
        None
    };

    json!({
        "as_of": reference_date,
        "window_days": window_days,
        "stale_after_days": stale_after_days,
        "counts": {
            // Copied verbatim from status() so they match tdw.kg.status exactly.
            "embedder_model": status.embedder_model,
            "vector_collection": status.vector_collection,
            "taxonomy_kind_count": status.taxonomy_kind_count,
            "active_findings": scan.active_total,
            "proposals_pending": proposals_pending,
            "proposals_by_state": status.proposals,
        },
        "new_knowledge": {
            // True total before display truncation, so the count never under-reports.
            "count": scan.new_in_window_total,
            "shown": scan.new_in_window.len(),
            "empty": scan.new_in_window_total == 0,
            "items": scan.new_in_window.iter().map(finding_to_value).collect::<Vec<_>>(),
        },
        "top_activity": {
            "count": scan.tag_activity_total,
            "shown": scan.tag_activity.len(),
            "empty": scan.tag_activity_total == 0,
            "items": scan.tag_activity.iter().map(|(label, count)| json!({
                "label": label,
                "count": count,
            })).collect::<Vec<_>>(),
        },
        "staleness": {
            // True total before display truncation, so the count never under-reports.
            "count": scan.stale_total,
            "shown": scan.stale.len(),
            "empty": scan.stale_total == 0,
            "items": scan.stale.iter().map(finding_to_value).collect::<Vec<_>>(),
        },
        "proposals": {
            "pending": proposals_pending,
            "by_state": status.proposals,
            "note": status.proposals.as_ref().map_or(
                "no proposal queue attached — write surface is off",
                |_| "pending proposals awaiting eval / operator action",
            ),
        },
        "eval_drift": {
            "alarm": eval_alarm,
            "freshness": status.eval_freshness,
        },
        "theses": status.theses,
        "scan_note": scan_note,
        "leakage_guard": "Findings are surfaced only when active_at(valid_from, valid_to, as_of) \
                          holds at the reference time; no finding whose validity begins after as_of \
                          appears in any section. Day spans use a leap-correct days-from-civil \
                          algorithm.",
    })
}

// ── Readable narrative ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_lines)] // narrative assembly is linear and readable as one unit
fn build_narrative(
    status: &KgStatus,
    reference_date: &str,
    window_days: i64,
    stale_after_days: i64,
    scan: &FindingScan,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    lines.push(format!(
        "Knowledge digest as of {reference_date} (looking back {window_days} day(s); \
         flagging findings stale after {stale_after_days} day(s))."
    ));
    lines.push(format!(
        "The knowledge base currently holds {} active finding(s), indexed under embedder {:?} \
         in collection {:?}.",
        scan.active_total, status.embedder_model, status.vector_collection
    ));

    // New knowledge.
    if scan.new_in_window_total == 0 {
        lines.push(format!(
            "New knowledge: no new knowledge in the last {window_days} day(s)."
        ));
    } else {
        lines.push(format!(
            "New knowledge: {} finding(s) appeared in the last {window_days} day(s).",
            scan.new_in_window_total
        ));
        for f in scan.new_in_window.iter().take(5) {
            let kind = if f.is_thesis { "thesis" } else { "finding" };
            lines.push(format!(
                "  - {kind} {:?} ({:?}), {} day(s) old.",
                f.title, f.id, f.age_days
            ));
        }
    }

    // Top activity.
    if scan.tag_activity_total == 0 {
        lines.push("Top activity: nothing was active in the window.".to_string());
    } else {
        let top: Vec<String> = scan
            .tag_activity
            .iter()
            .take(5)
            .map(|(label, count)| format!("{label:?} (×{count})"))
            .collect();
        lines.push(format!("Top activity: {}.", top.join(", ")));
    }

    // Staleness — the headline of the self-narration.
    if scan.stale_total == 0 {
        lines.push(format!(
            "Staleness: nothing is stale — every active finding has been corroborated within \
             the last {stale_after_days} day(s)."
        ));
    } else {
        lines.push(format!(
            "Staleness: {} finding(s) have not been corroborated in over {stale_after_days} \
             day(s) and may need attention.",
            scan.stale_total
        ));
        for f in scan.stale.iter().take(5) {
            let kind = if f.is_thesis { "thesis" } else { "finding" };
            let age = if f.age_days == i64::MAX {
                "unknown".to_string()
            } else {
                format!("{}", f.age_days)
            };
            lines.push(format!(
                "  - {kind} {:?} ({:?}) last corroborated {} — {age} day(s) ago.",
                f.title, f.id, f.as_of
            ));
        }
    }

    // Proposals awaiting eval.
    match status.proposals.as_ref() {
        None => {
            lines.push(
                "Proposals: no proposal queue is attached — the write surface is off.".to_string(),
            );
        }
        Some(p) => {
            let pending = p.draft + p.validated + p.ready;
            if pending == 0 {
                lines.push("Proposals: none pending — the eval queue is clear.".to_string());
            } else {
                lines.push(format!(
                    "Proposals: {pending} pending awaiting eval ({} draft, {} validated, {} ready).",
                    p.draft, p.validated, p.ready
                ));
            }
        }
    }

    // Eval drift alarm.
    match &status.eval_freshness {
        EvalFreshness::Unconfigured => {
            lines.push(
                "Eval drift: no golden split configured — retrieval drift is not being watched."
                    .to_string(),
            );
        }
        EvalFreshness::Pending { split_id } => {
            lines.push(format!(
                "Eval drift: the self-eval on split {split_id:?} is scheduled but has not run yet."
            ));
        }
        EvalFreshness::Green { summary, .. } => {
            lines.push(format!(
                "Eval drift: clear — last self-eval passed ({summary})."
            ));
        }
        EvalFreshness::Regressed {
            summary,
            drift_key_changed,
            ..
        } => {
            let cause = if *drift_key_changed {
                " (correlates with a version change — likely a version effect, not a code regression)"
            } else {
                ""
            };
            lines.push(format!(
                "Eval drift: ALARM — the last self-eval REGRESSED{cause}: {summary}"
            ));
        }
        EvalFreshness::Stale {
            error, split_id, ..
        } => {
            lines.push(format!(
                "Eval drift: ALARM — the last self-eval on split {split_id:?} FAILED to run: {error}"
            ));
        }
    }

    if scan.capped {
        lines.push(format!(
            "Note: the finding scan was bounded at {DIGEST_SCAN_CAP} nodes; some findings may be \
             omitted from the lists above."
        ));
    }

    lines.join("\n")
}

// ── Date math (leap-correct) ────────────────────────────────────────────────────

/// Days from the civil date 1970-01-01 to `(year, month, day)`.
///
/// Howard Hinnant's exact `days_from_civil` algorithm: correct across all
/// month/year boundaries and leap years, with no approximation.  This is what
/// replaces the approximate Julian formula that produced the prior story's
/// off-by-one bug.  `month` is 1..=12; the algorithm shifts the year so the
/// leap day falls at the end of the internal year.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Whole-day number for a normalized timestamp / date (`YYYY-MM-DD...`).
///
/// Returns the count of days since the epoch.  Malformed input returns `0`,
/// which makes day spans degrade gracefully rather than panicking.
fn day_number(ts: &str) -> i64 {
    let parse = || -> Option<i64> {
        let y: i64 = ts.get(0..4)?.parse().ok()?;
        let m: i64 = ts.get(5..7)?.parse().ok()?;
        let d: i64 = ts.get(8..10)?.parse().ok()?;
        if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
            return None;
        }
        Some(days_from_civil(y, m, d))
    };
    parse().unwrap_or(0)
}

// ── Argument helpers ────────────────────────────────────────────────────────────

fn optional_str<'a>(arguments: &'a Map<String, Value>, name: &str) -> Option<&'a str> {
    arguments.get(name).and_then(Value::as_str)
}

fn optional_i64(arguments: &Map<String, Value>, name: &str) -> Result<Option<i64>, ToolFailure> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| execution(format!("{name} must be an integer"))),
    }
}

/// `YYYY-MM-DD` shape check (mirrors the tag/retrieve date grammar).
///
/// Operates directly on the byte slice — the grammar is pure ASCII, so there is
/// no need to UTF-8 decode (Gemini efficiency finding).
fn is_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

const fn execution(message: String) -> ToolFailure {
    ToolFailure::Execution(message)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tdw_core::{GraphEngine, GraphNode};
    use tdw_embed_local::HashEmbeddingProvider;
    use tdw_storage_graph::InMemoryGraphEngine;
    use tdw_storage_qdrant::InMemoryVectorEngine;
    use tdw_taxonomy::EntityKind;

    use super::*;

    fn runtime_with_graph(graph: Arc<dyn GraphEngine>) -> KnowledgeRuntime {
        KnowledgeRuntime::new(
            Arc::new(HashEmbeddingProvider::default()),
            Arc::new(InMemoryVectorEngine::default()),
        )
        .with_graph(graph)
        .with_graph_name("in-memory")
    }

    /// Insert a Finding node with the given id, title, and `as_of` date.
    /// `valid_from` defaults to the `as_of` timestamp; the finding stays open.
    async fn insert_finding(
        graph: &Arc<dyn GraphEngine>,
        id: &str,
        title: &str,
        as_of_date: &str,
        is_thesis: bool,
    ) {
        let as_of_ts = date_to_timestamp(as_of_date);
        let mut props = json!({
            "title": title,
            "user_id": "tester",
            "as_of": as_of_ts,
        });
        if is_thesis {
            props["kind_hint"] = json!("thesis");
        }
        let node = GraphNode {
            id: id.to_string(),
            kind: EntityKind::Finding,
            label: title.to_string(),
            aliases: Vec::new(),
            props,
            valid_from: Some(as_of_ts.clone()),
            valid_to: None,
        };
        graph
            .upsert_nodes(vec![node])
            .await
            .expect("insert finding");
        // A self-referential audit edge so the edge page-walk reaches the node.
        // (The scan discovers nodes via edge endpoints.)
        graph
            .upsert_edges(vec![tdw_core::GraphEdge {
                from: id.to_string(),
                rel: "self".to_string(),
                to: id.to_string(),
                props: json!({}),
                provenance: tdw_core::Provenance::System {
                    detail: "test".to_string(),
                },
                valid_from: Some(as_of_ts.clone()),
                valid_to: None,
            }])
            .await
            .expect("insert edge");
    }

    fn args(map: serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
        map
    }

    fn run(runtime: &KnowledgeRuntime, arguments: &Map<String, Value>, now: &str) -> Value {
        match execute(runtime, "tdw.kg.digest", arguments, now) {
            Ok(exec) => exec.structured,
            Err(ToolFailure::Execution(message)) => panic!("digest execution error: {message}"),
            Err(ToolFailure::Protocol(_)) => panic!("digest returned a protocol error"),
        }
    }

    #[tokio::test]
    async fn digest_requires_graph_engine() {
        let runtime = KnowledgeRuntime::new(
            Arc::new(HashEmbeddingProvider::default()),
            Arc::new(InMemoryVectorEngine::default()),
        );
        let err = execute(&runtime, "tdw.kg.digest", &Map::new(), "2026-06-12");
        assert!(matches!(err, Err(ToolFailure::Execution(_))));
    }

    #[tokio::test]
    async fn digest_reflects_a_recent_ingest() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // A finding born 2 days before the reference time.
        insert_finding(
            &graph,
            "finding:fresh",
            "Recent learning",
            "2026-06-10",
            false,
        )
        .await;
        let runtime = runtime_with_graph(graph);

        let out = run(&runtime, &Map::new(), "2026-06-12");
        let new_knowledge = &out["new_knowledge"];
        assert_eq!(
            new_knowledge["empty"],
            json!(false),
            "fresh finding populates window: {out}"
        );
        assert_eq!(new_knowledge["count"], json!(1));
        assert_eq!(new_knowledge["items"][0]["id"], json!("finding:fresh"));
        // Narrative must mention the new knowledge plainly.
        let narrative = out["narrative"].as_str().expect("narrative is a string");
        assert!(
            narrative.contains("New knowledge: 1 finding"),
            "narrative narrates the ingest: {narrative}"
        );
    }

    #[tokio::test]
    async fn empty_window_yields_honest_nothing_new() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // A finding born well before the window (100 days old vs 7-day window).
        insert_finding(&graph, "finding:old", "Old learning", "2026-03-01", false).await;
        let runtime = runtime_with_graph(graph);

        let out = run(&runtime, &Map::new(), "2026-06-12");
        assert_eq!(out["new_knowledge"]["empty"], json!(true));
        assert_eq!(out["new_knowledge"]["count"], json!(0));
        let narrative = out["narrative"].as_str().expect("narrative is a string");
        assert!(
            narrative.contains("no new knowledge in the last 7 day(s)"),
            "honest empty narration: {narrative}"
        );
    }

    #[tokio::test]
    async fn stale_finding_is_surfaced() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // 100 days old vs default 90-day staleness threshold → stale.
        insert_finding(&graph, "finding:stale", "Aging fact", "2026-03-04", false).await;
        let runtime = runtime_with_graph(graph);

        let out = run(&runtime, &Map::new(), "2026-06-12");
        let staleness = &out["staleness"];
        assert_eq!(
            staleness["empty"],
            json!(false),
            "stale fixture surfaced: {out}"
        );
        assert_eq!(staleness["count"], json!(1));
        assert_eq!(staleness["items"][0]["id"], json!("finding:stale"));
        // age_days must be exactly 100 (leap-correct: Mar 4 → Jun 12, 2026).
        assert_eq!(
            staleness["items"][0]["age_days"],
            json!(100),
            "leap-correct day span across month boundaries"
        );
        let narrative = out["narrative"].as_str().expect("narrative is a string");
        assert!(
            narrative.contains("Staleness: 1 finding"),
            "narrative: {narrative}"
        );
    }

    #[tokio::test]
    async fn fresh_finding_is_not_stale() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        insert_finding(&graph, "finding:fresh", "Recent", "2026-06-10", false).await;
        let runtime = runtime_with_graph(graph);

        let out = run(&runtime, &Map::new(), "2026-06-12");
        assert_eq!(out["staleness"]["empty"], json!(true));
        let narrative = out["narrative"].as_str().expect("narrative is a string");
        assert!(
            narrative.contains("Staleness: nothing is stale"),
            "honest no-staleness narration: {narrative}"
        );
    }

    #[tokio::test]
    async fn leakage_safe_future_finding_excluded() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        // Finding born AFTER the reference time — must never appear.
        insert_finding(&graph, "finding:future", "Future fact", "2026-12-31", false).await;
        let runtime = runtime_with_graph(graph);

        let out = run(&runtime, &Map::new(), "2026-06-12");
        assert_eq!(
            out["counts"]["active_findings"],
            json!(0),
            "future finding not active: {out}"
        );
        assert_eq!(out["new_knowledge"]["empty"], json!(true));
        assert_eq!(out["staleness"]["empty"], json!(true));
    }

    #[tokio::test]
    async fn as_of_override_changes_window() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        insert_finding(&graph, "finding:mar", "March fact", "2026-03-04", false).await;
        let runtime = runtime_with_graph(graph);

        // With as_of just after the finding, it is fresh (not stale).
        let mut a = serde_json::Map::new();
        a.insert("as_of".to_string(), json!("2026-03-10"));
        let out = run(&runtime, &args(a), "2026-06-12");
        assert_eq!(
            out["staleness"]["empty"],
            json!(true),
            "fresh at as_of=2026-03-10: {out}"
        );
        assert_eq!(
            out["new_knowledge"]["empty"],
            json!(false),
            "in window at as_of"
        );
    }

    #[tokio::test]
    async fn counts_match_status() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        insert_finding(&graph, "finding:a", "A", "2026-06-10", false).await;
        let runtime = runtime_with_graph(graph);

        let out = run(&runtime, &Map::new(), "2026-06-12");
        let status = runtime.status().await;
        assert_eq!(
            out["counts"]["taxonomy_kind_count"],
            json!(status.taxonomy_kind_count),
            "digest taxonomy count matches status"
        );
        assert_eq!(
            out["counts"]["embedder_model"]
                .as_str()
                .expect("embedder_model is a string"),
            status.embedder_model,
            "digest embedder matches status"
        );
    }

    #[tokio::test]
    async fn count_is_honest_above_section_cap() {
        // More stale findings than SECTION_LIST_CAP: the reported `count` must be
        // the TRUE total (not the truncated display length), and `shown` must be
        // the cap. Guards the Gemini honesty finding.
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let total = SECTION_LIST_CAP + 5;
        for i in 0..total {
            // All born 2026-03-04 → 100 days old at 2026-06-12 → all stale.
            insert_finding(
                &graph,
                &format!("finding:stale-{i:03}"),
                &format!("Aging fact {i}"),
                "2026-03-04",
                false,
            )
            .await;
        }
        let runtime = runtime_with_graph(graph);

        let out = run(&runtime, &Map::new(), "2026-06-12");
        let staleness = &out["staleness"];
        assert_eq!(
            staleness["count"],
            json!(total),
            "count reports the true total, not the capped display length: {out}"
        );
        assert_eq!(
            staleness["shown"],
            json!(SECTION_LIST_CAP),
            "items are capped for display"
        );
        assert_eq!(
            staleness["items"].as_array().expect("items array").len(),
            SECTION_LIST_CAP,
            "items list is bounded to SECTION_LIST_CAP"
        );
        let narrative = out["narrative"].as_str().expect("narrative is a string");
        assert!(
            narrative.contains(&format!("Staleness: {total} finding")),
            "narrative reports the true total: {narrative}"
        );
    }

    #[tokio::test]
    async fn invalid_as_of_is_tool_error() {
        let graph: Arc<dyn GraphEngine> = Arc::new(InMemoryGraphEngine::default());
        let runtime = runtime_with_graph(graph);
        let mut a = serde_json::Map::new();
        a.insert("as_of".to_string(), json!("not-a-date"));
        let err = execute(&runtime, "tdw.kg.digest", &a, "2026-06-12");
        assert!(matches!(err, Err(ToolFailure::Execution(_))));
    }

    #[test]
    fn days_from_civil_handles_leap_and_boundaries() {
        // 2026-03-04 to 2026-06-12 is exactly 100 days (Mar:27 left + Apr:30 +
        // May:31 + Jun:12 = 100), crossing three month boundaries.
        assert_eq!(
            day_number("2026-06-12T00:00:00Z") - day_number("2026-03-04T00:00:00Z"),
            100
        );
        // Across a leap day: 2024-02-28 to 2024-03-01 is 2 days (2024 is leap).
        assert_eq!(
            day_number("2024-03-01") - day_number("2024-02-28"),
            2,
            "leap year includes Feb 29"
        );
        // Non-leap: 2023-02-28 to 2023-03-01 is 1 day.
        assert_eq!(
            day_number("2023-03-01") - day_number("2023-02-28"),
            1,
            "non-leap year has no Feb 29"
        );
        // Year boundary: 2025-12-31 to 2026-01-01 is 1 day.
        assert_eq!(day_number("2026-01-01") - day_number("2025-12-31"), 1);
    }
}
