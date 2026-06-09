#![forbid(unsafe_code)]
//! Per-owner watchlist policy layer.
//!
//! This crate is pure app logic over a normalized [`WatchlistEntry`] shape. It
//! does not perform any I/O and has no dependency on provider crates.
//!
//! # Core concepts
//!
//! * [`WatchlistEntry`] — the normalized shape every upstream watchlist row is
//!   mapped to before entering compose logic.
//! * [`WatchlistPolicy`] — tunable knobs: cap, max note length, symbol-casing
//!   toggle.
//! * [`compose`] — full pipeline: validate → normalize → dedup by symbol →
//!   cap, preserving stable input order.

#![deny(clippy::pedantic, clippy::nursery)]
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// WatchlistEntry
// ---------------------------------------------------------------------------

/// A single entry in an owner's watchlist.
///
/// All string fields are trimmed of leading/trailing whitespace during
/// [`normalize`]. `symbol` is (optionally) uppercased per
/// [`WatchlistPolicy::uppercase_symbols`].
///
/// `added_at_ms` is a Unix epoch in milliseconds (UTC). Using `i64` matches the
/// convention in `tdw-domain` (`QuoteSnapshot::ts_ms`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchlistEntry {
    /// Ticker symbol being watched. Must be non-blank (after trimming) to pass
    /// [`is_valid`].
    pub symbol: String,
    /// Timestamp the entry was added, as Unix epoch milliseconds (UTC).
    pub added_at_ms: i64,
    /// Optional free-text note, truncated to
    /// [`WatchlistPolicy::max_note_len`] Unicode scalar values during
    /// normalization. If truncated, an ellipsis (`…`) is appended. An empty
    /// note (after trimming) is dropped to `None` during [`normalize`].
    pub note: Option<String>,
}

impl WatchlistEntry {
    /// Construct a new [`WatchlistEntry`].
    ///
    /// No validation is performed here; call [`is_valid`] to check the
    /// invariants required by the compose pipeline.
    #[must_use]
    pub fn new(symbol: &str, added_at_ms: i64, note: Option<String>) -> Self {
        Self {
            symbol: symbol.to_string(),
            added_at_ms,
            note,
        }
    }
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors returned by the watchlist-compose pipeline.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WatchlistComposeError {
    /// A symbol was blank (empty after trimming).
    #[error("symbol is empty")]
    EmptySymbol,
    /// The composed watchlist exceeded [`WatchlistPolicy::max_symbols`].
    #[error("too many symbols: {0} exceeds cap")]
    TooManySymbols(usize),
    /// A note exceeded [`WatchlistPolicy::max_note_len`].
    #[error("note too long: {0} chars exceeds cap")]
    NoteTooLong(usize),
}

// ---------------------------------------------------------------------------
// WatchlistPolicy
// ---------------------------------------------------------------------------

/// Tunable knobs for the watchlist-compose pipeline.
///
/// Construct via [`Default`] and override individual fields as needed.
///
/// ```
/// use tdw_watchlist_compose::WatchlistPolicy;
///
/// let policy = WatchlistPolicy { max_symbols: 10, ..WatchlistPolicy::default() };
/// assert_eq!(policy.max_symbols, 10);
/// assert_eq!(policy.max_note_len, 280);
/// assert!(policy.uppercase_symbols);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchlistPolicy {
    /// Maximum number of entries to return from [`compose`]. Defaults to `50`.
    pub max_symbols: usize,
    /// Maximum number of Unicode scalar values kept in
    /// [`WatchlistEntry::note`] after [`normalize`]. Truncation appends `…`.
    /// Defaults to `280`.
    pub max_note_len: usize,
    /// When `true` (default), symbols are uppercased during [`normalize`] so
    /// that case-insensitive duplicates collide in [`dedup`].
    pub uppercase_symbols: bool,
}

impl Default for WatchlistPolicy {
    fn default() -> Self {
        Self {
            max_symbols: 50,
            max_note_len: 280,
            uppercase_symbols: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Returns `true` when `entry` satisfies the minimum structural invariants:
/// a non-blank `symbol` (after trimming).
///
/// Entries that fail this predicate are silently dropped by [`compose`].
#[must_use]
pub fn is_valid(entry: &WatchlistEntry) -> bool {
    !entry.symbol.trim().is_empty()
}

// ---------------------------------------------------------------------------
// Normalize
// ---------------------------------------------------------------------------

/// Return a normalized copy of `raw`.
///
/// Normalization performs:
/// 1. Trim whitespace from `symbol` and, when present, `note`.
/// 2. Uppercase `symbol` when [`WatchlistPolicy::uppercase_symbols`] is set.
/// 3. Truncate `note` to at most `max_note_len` Unicode scalar values.
///    Truncation always occurs on a Unicode scalar (char) boundary — never
///    mid-codepoint. When the note is truncated, the `…` (U+2026 HORIZONTAL
///    ELLIPSIS) character is appended.
/// 4. Drop the note to `None` when it is empty after trimming.
#[must_use]
pub fn normalize(raw: WatchlistEntry, policy: &WatchlistPolicy) -> WatchlistEntry {
    let trimmed = raw.symbol.trim();
    let symbol = if policy.uppercase_symbols {
        trimmed.to_ascii_uppercase()
    } else {
        trimmed.to_string()
    };

    let note = raw.note.and_then(|n| {
        let trimmed = n.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(truncate_note(trimmed, policy.max_note_len))
        }
    });

    WatchlistEntry {
        symbol,
        added_at_ms: raw.added_at_ms,
        note,
    }
}

/// Truncate `s` to at most `max_chars` Unicode scalar values.
///
/// If `s` is longer than `max_chars` chars, the returned string contains
/// exactly the first `max_chars` chars followed by `…` (U+2026). The split
/// always happens on a char boundary (guaranteed by iterating chars).
fn truncate_note(s: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_chars * 4 + 3));
    let mut truncated = false;
    for (count, ch) in s.chars().enumerate() {
        if count == max_chars {
            truncated = true;
            break;
        }
        out.push(ch);
    }
    if truncated {
        out.push('…');
    }
    out
}

// ---------------------------------------------------------------------------
// Dedup
// ---------------------------------------------------------------------------

/// Remove duplicate entries from `entries`, keyed by `symbol`.
///
/// The first occurrence of each symbol is kept; subsequent duplicates are
/// dropped. Order is otherwise preserved (stable).
///
/// # Ordering expectation
///
/// Dedup keys on the `symbol` field **verbatim**. Run [`normalize`] first so
/// that case-folded symbols (e.g. `"aapl"` and `"AAPL"`) collide; otherwise
/// they are treated as distinct. [`compose`] enforces this ordering.
#[must_use]
pub fn dedup(entries: Vec<WatchlistEntry>) -> Vec<WatchlistEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    entries
        .into_iter()
        .filter(|e| seen.insert(e.symbol.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// compose
// ---------------------------------------------------------------------------

/// Compose a watchlist into a normalized, deduplicated, capped list.
///
/// Pipeline: **validate** (drop invalid) → **normalize** → **dedup** by symbol
/// → **cap** at `policy.max_symbols` (keep the first `N`). The result preserves
/// stable input order throughout.
///
/// Returns an empty `Vec` when all entries are invalid or the input is empty.
#[must_use]
pub fn compose(entries: Vec<WatchlistEntry>, policy: &WatchlistPolicy) -> Vec<WatchlistEntry> {
    let normalized: Vec<WatchlistEntry> = entries
        .into_iter()
        .filter(is_valid)
        .map(|e| normalize(e, policy))
        .collect();

    let mut out = dedup(normalized);
    out.truncate(policy.max_symbols);
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(symbol: &str, ts: i64, note: Option<&str>) -> WatchlistEntry {
        WatchlistEntry::new(symbol, ts, note.map(str::to_string))
    }

    // ── is_valid ─────────────────────────────────────────────────────────────

    #[test]
    fn is_valid_accepts_non_blank_symbol() {
        let e = entry("AAPL", 0, None);
        assert!(is_valid(&e));
    }

    #[test]
    fn is_valid_rejects_empty_symbol() {
        let e = entry("", 0, None);
        assert!(!is_valid(&e));
    }

    #[test]
    fn is_valid_rejects_whitespace_only_symbol() {
        let e = entry("   ", 0, None);
        assert!(!is_valid(&e));
    }

    #[test]
    fn is_valid_ignores_note_length() {
        // is_valid only checks the symbol; note length is handled by normalize.
        let long = "x".repeat(10_000);
        let e = entry("AAPL", 0, Some(&long));
        assert!(is_valid(&e));
    }

    // ── normalize ────────────────────────────────────────────────────────────

    #[test]
    fn normalize_trims_and_uppercases_symbol() {
        let policy = WatchlistPolicy::default();
        let n = normalize(entry("  aapl  ", 0, None), &policy);
        assert_eq!(n.symbol, "AAPL");
    }

    #[test]
    fn normalize_preserves_case_when_uppercase_disabled() {
        let policy = WatchlistPolicy {
            uppercase_symbols: false,
            ..WatchlistPolicy::default()
        };
        let n = normalize(entry("  aapl  ", 0, None), &policy);
        assert_eq!(n.symbol, "aapl");
    }

    #[test]
    fn normalize_trims_note() {
        let policy = WatchlistPolicy::default();
        let n = normalize(entry("AAPL", 0, Some("  watch earnings  ")), &policy);
        assert_eq!(n.note.as_deref(), Some("watch earnings"));
    }

    #[test]
    fn normalize_drops_empty_note() {
        let policy = WatchlistPolicy::default();
        let n = normalize(entry("AAPL", 0, Some("   ")), &policy);
        assert_eq!(n.note, None);
    }

    #[test]
    fn normalize_truncates_note_at_char_limit() {
        let policy = WatchlistPolicy {
            max_note_len: 5,
            ..WatchlistPolicy::default()
        };
        let n = normalize(entry("AAPL", 0, Some("123456789")), &policy);
        assert_eq!(n.note.as_deref(), Some("12345\u{2026}"));
    }

    #[test]
    fn normalize_no_truncation_when_within_limit() {
        let policy = WatchlistPolicy {
            max_note_len: 10,
            ..WatchlistPolicy::default()
        };
        let n = normalize(entry("AAPL", 0, Some("Hello")), &policy);
        assert_eq!(n.note.as_deref(), Some("Hello"));
    }

    #[test]
    fn normalize_truncates_multibyte_unicode_on_char_boundary() {
        // Each of these chars is 3 bytes in UTF-8 (CJK block).
        let long_cjk = "中文测试内容过长检验"; // 9 chars
        let policy = WatchlistPolicy {
            max_note_len: 4,
            ..WatchlistPolicy::default()
        };
        let n = normalize(entry("AAPL", 0, Some(long_cjk)), &policy);
        let note = n.note.expect("note present");
        let chars: Vec<char> = note.chars().collect();
        assert_eq!(chars.last(), Some(&'…'));
        assert_eq!(chars.len(), 5); // 4 content chars + ellipsis
        // Verify it's valid UTF-8 (would have panicked on a bad boundary).
        assert!(std::str::from_utf8(note.as_bytes()).is_ok());
    }

    // ── dedup ────────────────────────────────────────────────────────────────

    #[test]
    fn dedup_removes_same_symbol_keeps_first() {
        let entries = vec![
            entry("AAPL", 100, Some("first")),
            entry("MSFT", 90, None),
            entry("AAPL", 80, Some("duplicate")),
        ];
        let out = dedup(entries);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].symbol, "AAPL");
        assert_eq!(out[0].note.as_deref(), Some("first"));
        assert_eq!(out[1].symbol, "MSFT");
    }

    #[test]
    fn dedup_preserves_order_of_unique_entries() {
        let entries = vec![
            entry("C", 3, None),
            entry("A", 1, None),
            entry("B", 2, None),
        ];
        let out = dedup(entries);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].symbol, "C");
        assert_eq!(out[1].symbol, "A");
        assert_eq!(out[2].symbol, "B");
    }

    #[test]
    fn dedup_collides_case_folded_symbols_after_normalize() {
        // Normalize first so "aapl" and "AAPL" both fold to "AAPL".
        let policy = WatchlistPolicy::default();
        let entries: Vec<WatchlistEntry> = vec![entry("aapl", 1, None), entry("AAPL", 2, None)]
            .into_iter()
            .map(|e| normalize(e, &policy))
            .collect();
        let out = dedup(entries);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].symbol, "AAPL");
        assert_eq!(out[0].added_at_ms, 1); // first occurrence kept
    }

    // ── compose ──────────────────────────────────────────────────────────────

    #[test]
    fn compose_drops_invalid_entries() {
        let policy = WatchlistPolicy::default();
        let entries = vec![
            entry("", 100, None),
            entry("AAPL", 200, None),
            entry("   ", 300, None),
        ];
        let out = compose(entries, &policy);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].symbol, "AAPL");
    }

    #[test]
    fn compose_dedups_case_insensitively() {
        let policy = WatchlistPolicy::default();
        let entries = vec![
            entry("aapl", 100, None),
            entry("AAPL", 200, None),
            entry("msft", 300, None),
        ];
        let out = compose(entries, &policy);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].symbol, "AAPL");
        assert_eq!(out[0].added_at_ms, 100); // first occurrence kept
        assert_eq!(out[1].symbol, "MSFT");
    }

    #[test]
    fn compose_caps_at_max_symbols_keeping_first() {
        let policy = WatchlistPolicy {
            max_symbols: 2,
            ..WatchlistPolicy::default()
        };
        let entries = vec![
            entry("AAPL", 1, None),
            entry("MSFT", 2, None),
            entry("GOOG", 3, None),
        ];
        let out = compose(entries, &policy);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].symbol, "AAPL");
        assert_eq!(out[1].symbol, "MSFT");
    }

    #[test]
    fn compose_preserves_stable_input_order() {
        let policy = WatchlistPolicy::default();
        let entries = vec![
            entry("TSLA", 1, None),
            entry("AAPL", 2, None),
            entry("MSFT", 3, None),
        ];
        let out = compose(entries, &policy);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].symbol, "TSLA");
        assert_eq!(out[1].symbol, "AAPL");
        assert_eq!(out[2].symbol, "MSFT");
    }

    #[test]
    fn compose_empty_input_returns_empty() {
        let policy = WatchlistPolicy::default();
        let out = compose(Vec::new(), &policy);
        assert!(out.is_empty());
    }

    #[test]
    fn compose_deterministic_for_same_input() {
        let policy = WatchlistPolicy::default();
        let entries = vec![
            entry("B", 200, Some("b")),
            entry("A", 300, Some("a")),
            entry("C", 100, None),
        ];
        let out1 = compose(entries.clone(), &policy);
        let out2 = compose(entries, &policy);
        assert_eq!(out1, out2);
    }

    // ── policy defaults ──────────────────────────────────────────────────────

    #[test]
    fn default_policy_values() {
        let policy = WatchlistPolicy::default();
        assert_eq!(policy.max_symbols, 50);
        assert_eq!(policy.max_note_len, 280);
        assert!(policy.uppercase_symbols);
    }
}
