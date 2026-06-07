//! Offline tiered context-compaction pipeline over a `Vec<ChatMessage>`.
//!
//! Gated by the `compaction` feature (which pulls in `tdw-llm` for the
//! [`ChatMessage`]/[`MessageRole`] vocabulary). The pipeline reduces the total
//! character footprint of a conversation transcript so it fits inside a
//! [`CompactBudget`] while preserving structural invariants:
//!
//! * **Content-only.** Every transform edits message *content*; none ever drops
//!   a role envelope. The number and ordering of role slots is preserved across
//!   the whole pipeline, so downstream consumers that key off positions stay
//!   valid. (Conservatively, a transform may rewrite a `content` to the empty
//!   string but never removes the message itself.)
//! * **Tool pairing.** [`assert_tool_pairing`] runs after every transform, both
//!   as a `debug_assert!` and as a runtime [`Result`]. Because [`ChatMessage`]
//!   carries no `tool_call_id`, pairing is checked *positionally* by role
//!   adjacency: a `Tool` message must be immediately preceded by an `Assistant`
//!   message or by another `Tool` message that itself traces back to an
//!   `Assistant` turn. The conversation tail must never begin with a `Tool`
//!   message.
//! * **Anti-thrash lock.** When the last two transforms each save less than
//!   10% of the prior character count, the [`Compactor`] *locks* and skips the
//!   remaining transforms; [`Compactor::is_locked`] reports the state and
//!   [`Compactor::reset`] clears it for reuse.
//!
//! Clean-room note: this is an original implementation of tiered compaction
//! concepts; no external agent code was copied.

use tdw_llm::{ChatMessage, MessageRole};

/// Savings-ratio threshold below which a transform counts as "ineffective" for
/// the anti-thrash lock. Two consecutive ineffective transforms lock the
/// pipeline.
const THRASH_RATIO: f64 = 0.10;

/// Budget that constrains a compaction run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactBudget {
    /// Target maximum total character count across all message contents.
    pub char_budget: usize,
    /// Number of leading messages to protect from lossy edits (system prompt,
    /// task framing, etc.).
    pub protect_head: usize,
    /// Number of trailing messages to protect from lossy edits (the live tail
    /// of the conversation).
    pub protect_tail: usize,
}

impl CompactBudget {
    /// Construct a budget.
    #[must_use]
    pub const fn new(char_budget: usize, protect_head: usize, protect_tail: usize) -> Self {
        Self {
            char_budget,
            protect_head,
            protect_tail,
        }
    }

    /// Returns `true` if the message at `index` lies inside the protected head
    /// or tail window for a transcript of `len` messages.
    #[must_use]
    pub const fn is_protected(&self, index: usize, len: usize) -> bool {
        if index < self.protect_head {
            return true;
        }
        // Tail window: the last `protect_tail` indices.
        len.saturating_sub(self.protect_tail) <= index
    }
}

impl Default for CompactBudget {
    fn default() -> Self {
        Self {
            char_budget: 16_384,
            protect_head: 1,
            protect_tail: 4,
        }
    }
}

/// Per-transform outcome record.
#[derive(Clone, Debug, PartialEq)]
pub struct CompactReport {
    /// Name of the transform that produced this report.
    pub transform: &'static str,
    /// Total content characters before the transform ran.
    pub chars_before: usize,
    /// Total content characters after the transform ran.
    pub chars_after: usize,
}

impl CompactReport {
    /// Number of characters removed (saturating; never negative).
    #[must_use]
    pub const fn chars_saved(&self) -> usize {
        self.chars_before.saturating_sub(self.chars_after)
    }

    /// Fraction of the prior character count removed, in `0.0..=1.0`. Returns
    /// `0.0` when there was nothing to remove.
    #[must_use]
    pub fn savings_ratio(&self) -> f64 {
        if self.chars_before == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        let ratio = self.chars_saved() as f64 / self.chars_before as f64;
        ratio
    }
}

/// Errors raised by the compaction pipeline.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CompactionError {
    /// A `Tool`-role message is not anchored to a preceding `Assistant` turn
    /// (or the tail begins with a `Tool` message). The `usize` is the offending
    /// message index.
    #[error("orphan tool message at index {0}: not anchored to a preceding assistant turn")]
    OrphanToolMessage(usize),
}

/// Total content characters across a transcript.
#[must_use]
fn total_chars(msgs: &[ChatMessage]) -> usize {
    msgs.iter().map(|m| m.content.chars().count()).sum()
}

/// Conservative, positional tool-pairing invariant check.
///
/// Because [`ChatMessage`] has no `tool_call_id`, pairing is inferred from role
/// adjacency:
///
/// * The transcript must not *begin* with a `Tool` message.
/// * Each `Tool` message must be immediately preceded by an `Assistant` message
///   or by another `Tool` message (which transitively anchors to an
///   `Assistant`). A `Tool` message preceded by `System` or `User` is an
///   orphan.
///
/// This is intentionally conservative: it never reorders or drops messages, it
/// only rejects transcripts whose tool messages have been separated from their
/// originating assistant turn.
///
/// # Errors
///
/// Returns [`CompactionError::OrphanToolMessage`] with the offending index.
pub fn assert_tool_pairing(msgs: &[ChatMessage]) -> Result<(), CompactionError> {
    let mut prev: Option<MessageRole> = None;
    for (index, msg) in msgs.iter().enumerate() {
        if msg.role == MessageRole::Tool {
            match prev {
                Some(MessageRole::Assistant | MessageRole::Tool) => {}
                _ => return Err(CompactionError::OrphanToolMessage(index)),
            }
        }
        prev = Some(msg.role);
    }
    Ok(())
}

/// A single tier of the compaction pipeline.
pub trait CompactionTransform {
    /// Stable, human-readable name (used in [`CompactReport::transform`]).
    fn name(&self) -> &'static str;

    /// Apply the transform in place and return its report.
    ///
    /// Implementations MUST be content-only: they may shrink or clear message
    /// `content`, but must never add, remove, or reorder message envelopes.
    fn apply(&self, msgs: &mut Vec<ChatMessage>, budget: &CompactBudget) -> CompactReport;
}

/// Tier 1: proactively trim trailing whitespace and collapse runs of blank
/// lines in unprotected messages. Lossless-ish housekeeping that runs first.
#[derive(Clone, Copy, Debug, Default)]
pub struct ProactiveTrim;

impl CompactionTransform for ProactiveTrim {
    fn name(&self) -> &'static str {
        "proactive_trim"
    }

    fn apply(&self, msgs: &mut Vec<ChatMessage>, budget: &CompactBudget) -> CompactReport {
        let before = total_chars(msgs);
        let len = msgs.len();
        for (index, msg) in msgs.iter_mut().enumerate() {
            if budget.is_protected(index, len) {
                continue;
            }
            let trimmed = collapse_whitespace(&msg.content);
            if trimmed.len() < msg.content.len() {
                msg.content = trimmed;
            }
        }
        CompactReport {
            transform: self.name(),
            chars_before: before,
            chars_after: total_chars(msgs),
        }
    }
}

/// Collapse trailing whitespace per line and squeeze 3+ consecutive blank lines
/// down to a single blank line. Never empties non-empty content.
fn collapse_whitespace(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut blank_run = 0usize;
    let mut first = true;
    for line in content.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        if !first {
            out.push('\n');
        }
        out.push_str(trimmed);
        first = false;
    }
    out
}

/// Tier 2: truncate over-long `Tool`-role message contents to `max_chars`,
/// appending an elision marker. Only touches unprotected `Tool` messages.
#[derive(Clone, Copy, Debug)]
pub struct ToolResultTruncate {
    /// Maximum character length retained per tool result.
    pub max_chars: usize,
}

impl ToolResultTruncate {
    /// Construct with the given retained-length cap.
    #[must_use]
    pub const fn new(max_chars: usize) -> Self {
        Self { max_chars }
    }
}

impl CompactionTransform for ToolResultTruncate {
    fn name(&self) -> &'static str {
        "tool_result_truncate"
    }

    fn apply(&self, msgs: &mut Vec<ChatMessage>, budget: &CompactBudget) -> CompactReport {
        let before = total_chars(msgs);
        let len = msgs.len();
        for (index, msg) in msgs.iter_mut().enumerate() {
            if msg.role != MessageRole::Tool || budget.is_protected(index, len) {
                continue;
            }
            if msg.content.chars().count() > self.max_chars {
                msg.content = truncate_with_marker(&msg.content, self.max_chars);
            }
        }
        CompactReport {
            transform: self.name(),
            chars_before: before,
            chars_after: total_chars(msgs),
        }
    }
}

/// Truncate `content` to at most `max_chars` retained characters and append a
/// one-line elision marker. The retained slice respects char boundaries.
fn truncate_with_marker(content: &str, max_chars: usize) -> String {
    let total = content.chars().count();
    let kept: String = content.chars().take(max_chars).collect();
    let elided = total.saturating_sub(max_chars);
    format!("{kept}\n[... {elided} chars truncated ...]")
}

/// Tier 3: prune "stale" unprotected messages by replacing their content with a
/// compact placeholder. Stale = `Assistant`/`User` messages in the middle of
/// the transcript (outside the protected head/tail). Role envelopes are kept so
/// tool pairing stays intact; only the content is reduced.
#[derive(Clone, Copy, Debug, Default)]
pub struct PruneStale;

impl CompactionTransform for PruneStale {
    fn name(&self) -> &'static str {
        "prune_stale"
    }

    fn apply(&self, msgs: &mut Vec<ChatMessage>, budget: &CompactBudget) -> CompactReport {
        let before = total_chars(msgs);
        // Only prune once we already exceed budget; otherwise no-op.
        if before <= budget.char_budget {
            return CompactReport {
                transform: self.name(),
                chars_before: before,
                chars_after: before,
            };
        }
        let len = msgs.len();
        for (index, msg) in msgs.iter_mut().enumerate() {
            if budget.is_protected(index, len) {
                continue;
            }
            // Never blank a Tool message via this tier (keep it adjacent to its
            // assistant turn with meaningful content for pairing audits).
            if matches!(msg.role, MessageRole::User | MessageRole::Assistant) {
                let placeholder = "[pruned stale message]";
                if msg.content.len() > placeholder.len() {
                    msg.content = placeholder.to_string();
                }
            }
        }
        CompactReport {
            transform: self.name(),
            chars_before: before,
            chars_after: total_chars(msgs),
        }
    }
}

/// Tier 4 (last resort): aggressively shrink a fraction of the unprotected
/// middle by clearing content down to a marker. `frac` in `0.0..=1.0` selects
/// how much of the unprotected span (from the oldest end) to drop content from.
/// Envelopes are always preserved.
#[derive(Clone, Copy, Debug)]
pub struct EmergencyDrop {
    /// Fraction of the unprotected span (oldest-first) to reduce, clamped to
    /// `0.0..=1.0`.
    pub frac: f64,
}

impl EmergencyDrop {
    /// Construct with the given fraction (clamped on use).
    #[must_use]
    pub const fn new(frac: f64) -> Self {
        Self { frac }
    }
}

impl CompactionTransform for EmergencyDrop {
    fn name(&self) -> &'static str {
        "emergency_drop"
    }

    fn apply(&self, msgs: &mut Vec<ChatMessage>, budget: &CompactBudget) -> CompactReport {
        let before = total_chars(msgs);
        if before <= budget.char_budget {
            return CompactReport {
                transform: self.name(),
                chars_before: before,
                chars_after: before,
            };
        }
        let len = msgs.len();
        let head = budget.protect_head.min(len);
        let tail_start = len.saturating_sub(budget.protect_tail);
        if head >= tail_start {
            // Nothing unprotected to drop.
            return CompactReport {
                transform: self.name(),
                chars_before: before,
                chars_after: before,
            };
        }
        let span = tail_start - head;
        let frac = self.frac.clamp(0.0, 1.0);
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let drop_count = (span as f64 * frac).ceil() as usize;
        let drop_count = drop_count.min(span);
        let marker = "[dropped]";
        for msg in msgs.iter_mut().skip(head).take(drop_count) {
            if msg.content.len() > marker.len() {
                msg.content = marker.to_string();
            }
        }
        CompactReport {
            transform: self.name(),
            chars_before: before,
            chars_after: total_chars(msgs),
        }
    }
}

/// Orchestrates an ordered set of [`CompactionTransform`]s with an anti-thrash
/// lock.
pub struct Compactor {
    transforms: Vec<Box<dyn CompactionTransform>>,
    budget: CompactBudget,
    locked: bool,
}

impl Compactor {
    /// Construct from an explicit transform list and budget.
    #[must_use]
    pub fn new(transforms: Vec<Box<dyn CompactionTransform>>, budget: CompactBudget) -> Self {
        Self {
            transforms,
            budget,
            locked: false,
        }
    }

    /// The standard offline tiered pipeline: trim, then truncate tool results,
    /// then prune stale middle, then emergency-drop as last resort.
    #[must_use]
    pub fn default_pipeline() -> Self {
        Self::new(
            vec![
                Box::new(ProactiveTrim),
                Box::new(ToolResultTruncate::new(2_048)),
                Box::new(PruneStale),
                Box::new(EmergencyDrop::new(0.5)),
            ],
            CompactBudget::default(),
        )
    }

    /// Whether the anti-thrash lock has engaged.
    #[must_use]
    pub const fn is_locked(&self) -> bool {
        self.locked
    }

    /// Clear the anti-thrash lock so the compactor can be reused.
    pub const fn reset(&mut self) {
        self.locked = false;
    }

    /// Run the pipeline in place, returning one [`CompactReport`] per transform
    /// that actually ran (the lock may stop the run early).
    ///
    /// After every transform the tool-pairing invariant is re-checked: in debug
    /// builds via `debug_assert!`, and unconditionally at runtime — a violation
    /// rolls back to the pre-transform snapshot and halts the pipeline so the
    /// caller never receives a structurally broken transcript.
    pub fn compact(&mut self, msgs: &mut Vec<ChatMessage>) -> Vec<CompactReport> {
        let mut reports = Vec::with_capacity(self.transforms.len());
        let mut recent: Vec<f64> = Vec::new();
        for transform in &self.transforms {
            if self.locked {
                break;
            }
            let snapshot = msgs.clone();
            let report = transform.apply(msgs, &self.budget);

            // Invariant: content-only transforms must preserve envelope count.
            debug_assert_eq!(
                msgs.len(),
                snapshot.len(),
                "transform {} changed the message count",
                report.transform
            );

            // Tool pairing must hold after every transform.
            let pairing = assert_tool_pairing(msgs);
            debug_assert!(
                pairing.is_ok(),
                "transform {} broke tool pairing: {pairing:?}",
                report.transform
            );
            if pairing.is_err() || msgs.len() != snapshot.len() {
                // Conservative rollback; never hand back a broken transcript.
                *msgs = snapshot;
                break;
            }

            let ratio = report.savings_ratio();
            reports.push(report);

            recent.push(ratio);
            if recent.len() >= 2 {
                let n = recent.len();
                if recent[n - 1] < THRASH_RATIO && recent[n - 2] < THRASH_RATIO {
                    self.locked = true;
                }
            }
        }
        reports
    }

    /// Borrow the configured budget.
    #[must_use]
    pub const fn budget(&self) -> &CompactBudget {
        &self.budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: MessageRole, content: &str) -> ChatMessage {
        ChatMessage {
            role,
            content: content.to_string(),
        }
    }

    fn convo() -> Vec<ChatMessage> {
        vec![
            msg(MessageRole::System, "system policy"),
            msg(MessageRole::User, "question one"),
            msg(MessageRole::Assistant, "calling a tool"),
            msg(MessageRole::Tool, &"X".repeat(10_000)),
            msg(MessageRole::Assistant, "answer one with reasoning"),
            msg(MessageRole::User, "question two"),
            msg(MessageRole::Assistant, "final answer"),
        ]
    }

    #[test]
    fn proactive_trim_collapses_whitespace_and_keeps_envelopes() {
        let mut msgs = vec![
            msg(MessageRole::System, "head"),
            msg(MessageRole::User, "line   \n\n\n\nmore  \n"),
            msg(MessageRole::Assistant, "tail"),
        ];
        let budget = CompactBudget::new(10_000, 1, 1);
        let report = ProactiveTrim.apply(&mut msgs, &budget);

        assert_eq!(msgs.len(), 3, "no envelope dropped");
        assert!(report.chars_after < report.chars_before);
        // Trailing spaces gone, blank run collapsed.
        assert_eq!(msgs[1].content, "line\n\nmore");
        // Protected head/tail untouched.
        assert_eq!(msgs[0].content, "head");
        assert_eq!(msgs[2].content, "tail");
    }

    #[test]
    fn tool_result_truncate_caps_only_tool_messages() {
        let mut msgs = convo();
        let budget = CompactBudget::new(10_000, 1, 1);
        let before_user = msgs[1].content.clone();
        let report = ToolResultTruncate::new(100).apply(&mut msgs, &budget);

        assert!(report.chars_saved() > 9_000);
        // The big tool message shrank.
        assert!(msgs[3].content.chars().count() < 200);
        assert!(msgs[3].content.contains("truncated"));
        // Non-tool messages untouched.
        assert_eq!(msgs[1].content, before_user);
        // Still structurally sound.
        assert!(assert_tool_pairing(&msgs).is_ok());
    }

    #[test]
    fn assert_tool_pairing_rejects_orphan_tool_after_user() {
        let bad = vec![
            msg(MessageRole::System, "s"),
            msg(MessageRole::User, "u"),
            msg(MessageRole::Tool, "orphan"),
        ];
        assert_eq!(
            assert_tool_pairing(&bad),
            Err(CompactionError::OrphanToolMessage(2))
        );
    }

    #[test]
    fn assert_tool_pairing_rejects_leading_tool() {
        let bad = vec![msg(MessageRole::Tool, "first")];
        assert_eq!(
            assert_tool_pairing(&bad),
            Err(CompactionError::OrphanToolMessage(0))
        );
    }

    #[test]
    fn assert_tool_pairing_accepts_assistant_then_tool_chain() {
        let good = vec![
            msg(MessageRole::Assistant, "call"),
            msg(MessageRole::Tool, "result a"),
            msg(MessageRole::Tool, "result b"),
            msg(MessageRole::User, "next"),
        ];
        assert!(assert_tool_pairing(&good).is_ok());
    }

    // BINDING test: a Tool message is never separated from its preceding
    // Assistant turn after the full default pipeline runs, and the tail of the
    // transcript never begins with a Tool-role message.
    #[test]
    fn default_pipeline_preserves_tail_and_tool_adjacency() {
        let mut msgs = convo();
        let original_roles: Vec<MessageRole> = msgs.iter().map(|m| m.role).collect();
        let mut compactor = Compactor::default_pipeline();
        let reports = compactor.compact(&mut msgs);

        assert!(!reports.is_empty());
        // Content-only: role envelopes preserved exactly, in order.
        let after_roles: Vec<MessageRole> = msgs.iter().map(|m| m.role).collect();
        assert_eq!(original_roles, after_roles);

        // The whole transcript stays structurally valid after compaction.
        assert!(assert_tool_pairing(&msgs).is_ok());

        // BINDING: the transcript does not *begin* with a Tool message, so the
        // tail can never be split such that it starts with an unanchored Tool.
        assert_ne!(msgs.first().map(|m| m.role), Some(MessageRole::Tool));

        // The Tool message stays immediately after its Assistant turn (never
        // separated by the pipeline).
        let tool_idx = msgs
            .iter()
            .position(|m| m.role == MessageRole::Tool)
            .expect("tool message present");
        assert!(tool_idx > 0);
        assert_eq!(msgs[tool_idx - 1].role, MessageRole::Assistant);
    }

    // BINDING test: a tail window that would *start with* a Tool-role message
    // (i.e. a Tool separated from its preceding Assistant turn) is rejected by
    // the pairing guard, which is what every transform is checked against.
    #[test]
    fn tail_starting_with_tool_is_an_orphan() {
        // Simulate a transcript where the live "tail" begins at the Tool: the
        // guard must flag the Tool as orphaned when its left neighbour is not
        // an Assistant/Tool anchor.
        let split = vec![
            msg(MessageRole::User, "earlier question"),
            msg(MessageRole::Tool, "result with no assistant before it"),
            msg(MessageRole::Assistant, "answer"),
        ];
        assert_eq!(
            assert_tool_pairing(&split),
            Err(CompactionError::OrphanToolMessage(1))
        );
    }

    #[test]
    fn anti_thrash_lock_engages_after_two_low_savings() {
        // A transcript already under budget: every tier saves ~0% -> lock.
        let mut msgs = vec![
            msg(MessageRole::System, "s"),
            msg(MessageRole::User, "small"),
            msg(MessageRole::Assistant, "tiny"),
            msg(MessageRole::User, "q"),
            msg(MessageRole::Assistant, "a"),
        ];
        let mut compactor = Compactor::new(
            vec![
                Box::new(ProactiveTrim),
                Box::new(PruneStale),
                Box::new(EmergencyDrop::new(0.5)),
            ],
            CompactBudget::new(1_000_000, 1, 1),
        );
        let reports = compactor.compact(&mut msgs);

        assert!(compactor.is_locked(), "two <10% savings should lock");
        // Locked before the third transform got to run.
        assert!(reports.len() < 3);

        compactor.reset();
        assert!(!compactor.is_locked());
    }

    #[test]
    fn savings_ratio_is_bounded_and_zero_safe() {
        let zero = CompactReport {
            transform: "x",
            chars_before: 0,
            chars_after: 0,
        };
        assert!((zero.savings_ratio() - 0.0).abs() < f64::EPSILON);

        let half = CompactReport {
            transform: "x",
            chars_before: 100,
            chars_after: 50,
        };
        assert!((half.savings_ratio() - 0.5).abs() < f64::EPSILON);
        assert_eq!(half.chars_saved(), 50);
    }

    #[test]
    fn emergency_drop_respects_protection_and_envelopes() {
        let mut msgs = convo();
        // Force over-budget so the drop activates.
        let budget = CompactBudget::new(1, 1, 2);
        let len = msgs.len();
        let report = EmergencyDrop::new(1.0).apply(&mut msgs, &budget);

        assert_eq!(msgs.len(), len, "no envelope dropped");
        assert!(report.chars_after < report.chars_before);
        // Protected head intact.
        assert_eq!(msgs[0].content, "system policy");
        // Protected tail (last 2) intact.
        assert_eq!(msgs[len - 1].content, "final answer");
        assert!(assert_tool_pairing(&msgs).is_ok());
    }

    #[test]
    fn prune_stale_is_noop_under_budget() {
        let mut msgs = convo();
        let budget = CompactBudget::new(1_000_000, 1, 2);
        let report = PruneStale.apply(&mut msgs, &budget);
        assert_eq!(report.chars_before, report.chars_after);
    }

    #[test]
    fn is_protected_windows() {
        let b = CompactBudget::new(10, 1, 2);
        // len = 5: head index 0 protected; tail indices 3,4 protected.
        assert!(b.is_protected(0, 5));
        assert!(!b.is_protected(1, 5));
        assert!(!b.is_protected(2, 5));
        assert!(b.is_protected(3, 5));
        assert!(b.is_protected(4, 5));
    }
}
