//! Additive, opt-in stuck-loop guard for repeated tool invocations.
//!
//! [`LoopGuard`] keeps a small ring buffer of recent *call signatures* — a `u64` hash of a
//! tool name plus its canonicalized arguments — and graduates a verdict from [`GuardDecision::Allow`]
//! through [`GuardDecision::Warn`] to [`GuardDecision::Refuse`] when the same (or an A-B-A-B
//! ping-ponging) call repeats often enough to look like a stuck loop.
//!
//! This is a *hint*, not a hard limit: the guard never executes anything itself. A caller
//! threads [`LoopGuard::observe`] in front of [`crate::ToolExecutor::execute`] and decides what to
//! do with the verdict.
//!
//! # Signature stability
//!
//! Signatures are produced by [`std::hash::DefaultHasher`] over the tool name and canonicalized
//! arguments. `DefaultHasher` is **not** guaranteed stable across Rust versions, platforms, or
//! even process runs (its seed may change). The signature is therefore **in-process only** and
//! **MUST NOT be persisted** or compared across processes. It is deliberately *not* related to,
//! and must not be confused with, any hash-chained receipt-log hash elsewhere in the tree.

use std::collections::{BTreeSet, VecDeque};
use std::hash::{Hash as _, Hasher as _};

use serde_json::Value;

/// Default number of recent signatures retained in the ring buffer.
///
/// Four is the minimum that still detects an A-B-A-B ping-pong (two distinct signatures
/// alternating) over "the last 4".
const DEFAULT_CAPACITY: usize = 4;
/// Default count of identical-signature repeats (within the window) that escalates past
/// [`GuardDecision::Allow`].
///
/// Conservative: a single repeat is normal (idempotent retry, pagination poll); escalation
/// only begins once a signature has been seen enough times to look stuck.
const DEFAULT_REPEAT_THRESHOLD: u32 = 3;
/// Default number of [`GuardDecision::Warn`] verdicts emitted before escalating to
/// [`GuardDecision::Refuse`].
const DEFAULT_MAX_WARNINGS: u32 = 2;

/// The guard's verdict for a single observed call.
///
/// Returned by [`LoopGuard::observe`]; the caller decides how to act on it. The guard is purely
/// advisory and performs no side effects of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardDecision {
    /// Nothing loop-like detected; proceed.
    Allow,
    /// A repeat pattern is forming. `repeats` is how many times the current signature has been
    /// seen within the window; `hint` is a human-readable suggestion.
    Warn {
        /// How many times the current signature appears in the retained window (including this
        /// observation).
        repeats: u32,
        /// Human-readable suggestion describing the detected pattern.
        hint: String,
    },
    /// The loop persisted past `max_warnings`; the caller should stop re-issuing this call.
    /// `signature` is the in-process signature that is looping (debug-only; not stable).
    Refuse {
        /// The in-process signature that triggered the refusal. Not version-stable; do not
        /// persist.
        signature: u64,
    },
}

/// A ring-buffer stuck-loop detector over canonical call signatures.
///
/// Construct with [`LoopGuard::new`] (conservative defaults) or [`LoopGuard::builder`] for
/// explicit thresholds, then call [`LoopGuard::observe`] once per intended tool call. Exempt
/// tools (registered via [`LoopGuardBuilder::with_exempt`]) always return [`GuardDecision::Allow`]
/// and never consume window state — appropriate for legitimately repetitive polling tools.
///
/// The default exemption set is **empty**: no tool names are hardcoded, and a fresh guard treats
/// every tool identically. Callers opt specific tools out explicitly.
#[derive(Clone, Debug)]
pub struct LoopGuard {
    /// Recent signatures, oldest at the front. Bounded to `capacity`.
    window: VecDeque<u64>,
    /// Maximum number of signatures retained in `window`.
    capacity: usize,
    /// Identical-signature occurrences within the window required to leave `Allow`.
    repeat_threshold: u32,
    /// Number of `Warn` verdicts emitted before escalating to `Refuse`.
    max_warnings: u32,
    /// Warnings already emitted for the currently looping signature.
    warnings_emitted: u32,
    /// The signature warnings are currently being counted for, if any.
    warned_signature: Option<u64>,
    /// Tool names that bypass the guard entirely. Empty by default.
    exempt: BTreeSet<String>,
}

impl LoopGuard {
    /// A guard with conservative defaults and an **empty** exemption set.
    #[must_use]
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Start building a guard with custom thresholds and exemptions.
    #[must_use]
    pub const fn builder() -> LoopGuardBuilder {
        LoopGuardBuilder::new()
    }

    /// Compute the in-process signature of a `(tool, args)` call.
    ///
    /// The hash covers the tool name and a canonicalized form of `args` in which object keys are
    /// recursively sorted, so semantically identical JSON with differently ordered keys yields the
    /// same signature. Backed by [`std::hash::DefaultHasher`]: in-process only, **not**
    /// version-stable, **must not** be persisted.
    #[must_use]
    pub fn signature(tool: &str, args: &Value) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tool.hash(&mut hasher);
        hash_canonical(args, &mut hasher);
        hasher.finish()
    }

    /// Observe an intended `(tool, args)` call and return the guard's verdict.
    ///
    /// Exempt tools always return [`GuardDecision::Allow`] without recording window state. For
    /// every other tool the signature is recorded, then the in-window repeat count drives the
    /// graduated `Allow -> Warn -> Refuse` decision.
    pub fn observe(&mut self, tool: &str, args: &Value) -> GuardDecision {
        if self.exempt.contains(tool) {
            return GuardDecision::Allow;
        }

        let signature = Self::signature(tool, args);
        self.push(signature);
        let repeats = self.count(signature);

        if repeats < self.repeat_threshold {
            // A different signature breaks an in-progress warning streak.
            if self.warned_signature != Some(signature) {
                self.reset_warnings();
            }
            return GuardDecision::Allow;
        }

        // We are at/over the repeat threshold: this signature looks stuck.
        if self.warned_signature != Some(signature) {
            self.warned_signature = Some(signature);
            self.warnings_emitted = 0;
        }

        if self.warnings_emitted >= self.max_warnings {
            return GuardDecision::Refuse { signature };
        }

        self.warnings_emitted += 1;
        GuardDecision::Warn {
            repeats,
            hint: format!(
                "tool '{tool}' has repeated an identical call {repeats} times within the last \
                 {capacity}; vary the arguments or stop retrying",
                capacity = self.capacity
            ),
        }
    }

    /// Push a signature into the bounded window, evicting the oldest if full.
    fn push(&mut self, signature: u64) {
        if self.window.len() == self.capacity {
            self.window.pop_front();
        }
        self.window.push_back(signature);
    }

    /// Count how many times `signature` appears in the retained window.
    fn count(&self, signature: u64) -> u32 {
        u32::try_from(self.window.iter().filter(|&&sig| sig == signature).count())
            .unwrap_or(u32::MAX)
    }

    /// Clear any in-progress warning streak.
    const fn reset_warnings(&mut self) {
        self.warned_signature = None;
        self.warnings_emitted = 0;
    }
}

impl Default for LoopGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for [`LoopGuard`].
///
/// Defaults are conservative ([`DEFAULT_CAPACITY`], [`DEFAULT_REPEAT_THRESHOLD`],
/// [`DEFAULT_MAX_WARNINGS`]) and the exemption set is empty.
#[derive(Clone, Debug)]
pub struct LoopGuardBuilder {
    capacity: usize,
    repeat_threshold: u32,
    max_warnings: u32,
    exempt: BTreeSet<String>,
}

impl LoopGuardBuilder {
    /// A builder pre-loaded with the conservative defaults and an empty exemption set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            capacity: DEFAULT_CAPACITY,
            repeat_threshold: DEFAULT_REPEAT_THRESHOLD,
            max_warnings: DEFAULT_MAX_WARNINGS,
            exempt: BTreeSet::new(),
        }
    }

    /// Override the ring-buffer capacity (number of recent signatures retained).
    ///
    /// Clamped to a minimum of 1; a zero-length window cannot detect anything.
    #[must_use]
    pub fn capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity.max(1);
        self
    }

    /// Override the identical-signature repeat count that escalates past `Allow`.
    ///
    /// Clamped to a minimum of 1.
    #[must_use]
    pub fn repeat_threshold(mut self, repeat_threshold: u32) -> Self {
        self.repeat_threshold = repeat_threshold.max(1);
        self
    }

    /// Override the number of `Warn` verdicts emitted before `Refuse`.
    #[must_use]
    pub const fn max_warnings(mut self, max_warnings: u32) -> Self {
        self.max_warnings = max_warnings;
        self
    }

    /// Exempt the given tool names from the guard (they always return `Allow`).
    ///
    /// Accumulates across calls. There are no hardcoded defaults; this is the only way a tool
    /// becomes exempt.
    #[must_use]
    pub fn with_exempt<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.exempt.extend(names.into_iter().map(Into::into));
        self
    }

    /// Finalize the [`LoopGuard`].
    #[must_use]
    pub fn build(self) -> LoopGuard {
        LoopGuard {
            window: VecDeque::with_capacity(self.capacity),
            capacity: self.capacity,
            repeat_threshold: self.repeat_threshold,
            max_warnings: self.max_warnings,
            warnings_emitted: 0,
            warned_signature: None,
            exempt: self.exempt,
        }
    }
}

impl Default for LoopGuardBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Feed a JSON value into `hasher` in a canonical, key-order-independent way.
///
/// Objects are hashed with their keys visited in sorted order so that two objects differing only
/// in key insertion order produce identical hashes. A small per-variant tag is mixed in to keep
/// distinct JSON shapes (e.g. the string `"1"` vs the number `1`) from colliding.
fn hash_canonical(value: &Value, hasher: &mut impl std::hash::Hasher) {
    match value {
        Value::Null => 0u8.hash(hasher),
        Value::Bool(b) => {
            1u8.hash(hasher);
            b.hash(hasher);
        }
        Value::Number(n) => {
            2u8.hash(hasher);
            // `Number`'s textual form is canonical for our purposes and avoids f64 NaN/`Hash`
            // pitfalls.
            n.to_string().hash(hasher);
        }
        Value::String(s) => {
            3u8.hash(hasher);
            s.hash(hasher);
        }
        Value::Array(items) => {
            4u8.hash(hasher);
            items.len().hash(hasher);
            for item in items {
                hash_canonical(item, hasher);
            }
        }
        Value::Object(map) => {
            5u8.hash(hasher);
            map.len().hash(hasher);
            // Visit keys in sorted order for order-independence. `serde_json` without the
            // `preserve_order` feature already orders keys, but sorting explicitly makes the
            // canonicalization correct regardless of that build flag.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            for key in keys {
                key.hash(hasher);
                hash_canonical(&map[key], hasher);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normal_non_repeating_sequence_always_allows() {
        let mut guard = LoopGuard::new();
        // A varied sequence of distinct calls must never escalate.
        for i in 0..20 {
            let decision = guard.observe("tool.search", &json!({ "page": i }));
            assert_eq!(decision, GuardDecision::Allow, "iteration {i} should allow");
        }
    }

    #[test]
    fn exempt_tool_poll_loop_always_allows() {
        let mut guard = LoopGuard::builder()
            .with_exempt(["tool.poll.status"])
            .build();
        // An exempt tool hammering the identical call must never escalate.
        for _ in 0..50 {
            let decision = guard.observe("tool.poll.status", &json!({ "id": "job-1" }));
            assert_eq!(decision, GuardDecision::Allow);
        }
    }

    #[test]
    fn graduated_allow_warn_refuse_on_identical_repeats() {
        // capacity large enough to retain all repeats; threshold 3, one warning then refuse.
        let mut guard = LoopGuard::builder()
            .capacity(8)
            .repeat_threshold(3)
            .max_warnings(1)
            .build();
        let args = json!({ "q": "stuck" });

        // Repeats 1 and 2 are below threshold -> Allow.
        assert_eq!(guard.observe("tool.x", &args), GuardDecision::Allow);
        assert_eq!(guard.observe("tool.x", &args), GuardDecision::Allow);

        // Repeat 3 hits the threshold -> first Warn.
        match guard.observe("tool.x", &args) {
            GuardDecision::Warn { repeats, hint } => {
                assert_eq!(repeats, 3);
                assert!(hint.contains("tool.x"));
            }
            other => panic!("expected Warn, got {other:?}"),
        }

        // Repeat 4: max_warnings (1) already emitted -> Refuse.
        match guard.observe("tool.x", &args) {
            GuardDecision::Refuse { signature } => {
                assert_eq!(signature, LoopGuard::signature("tool.x", &args));
            }
            other => panic!("expected Refuse, got {other:?}"),
        }

        // It stays refused while the loop persists.
        assert!(matches!(
            guard.observe("tool.x", &args),
            GuardDecision::Refuse { .. }
        ));
    }

    #[test]
    fn graduated_emits_multiple_warnings_before_refuse() {
        let mut guard = LoopGuard::builder()
            .capacity(16)
            .repeat_threshold(2)
            .max_warnings(2)
            .build();
        let args = json!({ "k": 1 });

        assert_eq!(guard.observe("t", &args), GuardDecision::Allow); // repeat 1
        assert!(matches!(
            guard.observe("t", &args),
            GuardDecision::Warn { repeats: 2, .. }
        )); // warning 1
        assert!(matches!(
            guard.observe("t", &args),
            GuardDecision::Warn { repeats: 3, .. }
        )); // warning 2
        assert!(matches!(
            guard.observe("t", &args),
            GuardDecision::Refuse { .. }
        )); // exhausted
    }

    #[test]
    fn ping_pong_a_b_a_b_over_last_four_is_detected() {
        // Default capacity 4. Alternating A-B-A-B means each signature appears twice within the
        // last 4 observations. With repeat_threshold 2 the second occurrence of each escalates.
        let mut guard = LoopGuard::builder()
            .capacity(4)
            .repeat_threshold(2)
            .max_warnings(1)
            .build();
        let a = json!({ "side": "a" });
        let b = json!({ "side": "b" });

        // window: [A]
        assert_eq!(guard.observe("tool.pp", &a), GuardDecision::Allow);
        // window: [A, B]
        assert_eq!(guard.observe("tool.pp", &b), GuardDecision::Allow);
        // window: [A, B, A] -> A appears twice -> threshold 2 -> Warn
        assert!(matches!(
            guard.observe("tool.pp", &a),
            GuardDecision::Warn { repeats: 2, .. }
        ));
        // window: [A, B, A, B] -> B now appears twice. A new signature resets the warn streak,
        // so this is B's first warning.
        assert!(matches!(
            guard.observe("tool.pp", &b),
            GuardDecision::Warn { repeats: 2, .. }
        ));
    }

    #[test]
    fn canonicalizer_is_key_order_independent() {
        // Two objects with the same content but different key insertion order must share a
        // signature.
        let lhs: Value = serde_json::from_str(r#"{"a":1,"b":{"x":1,"y":2}}"#).expect("parse lhs");
        let rhs: Value = serde_json::from_str(r#"{"b":{"y":2,"x":1},"a":1}"#).expect("parse rhs");
        assert_eq!(
            LoopGuard::signature("tool.k", &lhs),
            LoopGuard::signature("tool.k", &rhs),
        );
    }

    #[test]
    fn signature_distinguishes_tool_name_and_args() {
        let args = json!({ "v": 1 });
        assert_ne!(
            LoopGuard::signature("tool.a", &args),
            LoopGuard::signature("tool.b", &args),
        );
        assert_ne!(
            LoopGuard::signature("tool.a", &json!({ "v": 1 })),
            LoopGuard::signature("tool.a", &json!({ "v": 2 })),
        );
    }

    #[test]
    fn signature_distinguishes_string_from_number() {
        // The per-variant tag keeps "1" (string) from colliding with 1 (number).
        assert_ne!(
            LoopGuard::signature("t", &json!({ "v": "1" })),
            LoopGuard::signature("t", &json!({ "v": 1 })),
        );
    }

    #[test]
    fn different_tools_do_not_alias_each_other() {
        // Interleaving two distinct tools (each non-repeating) must stay Allow.
        let mut guard = LoopGuard::new();
        for i in 0..10 {
            assert_eq!(
                guard.observe("tool.one", &json!({ "i": i })),
                GuardDecision::Allow
            );
            assert_eq!(
                guard.observe("tool.two", &json!({ "i": i })),
                GuardDecision::Allow
            );
        }
    }
}
