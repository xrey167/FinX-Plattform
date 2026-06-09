#![forbid(unsafe_code)]
//! News-aggregation policy layer.
//!
//! This crate is pure app logic over a normalized [`Article`] shape. It does
//! not perform any I/O and has no dependency on provider crates.
//!
//! # Core concepts
//!
//! * [`Article`] — the normalized wire shape every upstream feed is mapped to
//!   before entering compose logic.
//! * [`ComposePolicy`] — tunable knobs: cap, max summary length, dedup toggle.
//! * [`compose`] — single-list variant: validate → normalize → dedup →
//!   sort newest-first → cap.
//! * [`compose_for_symbols`] — multi-symbol variant using round-robin
//!   distribution so no single symbol dominates the result.

#![deny(clippy::pedantic, clippy::nursery)]
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Article
// ---------------------------------------------------------------------------

/// A normalized news article suitable for display or downstream processing.
///
/// All string fields are trimmed of leading/trailing whitespace during
/// [`normalize`]. `symbols` are uppercased and deduplicated in-place.
///
/// `published_ts_ms` is a Unix epoch in milliseconds (UTC). Using `i64`
/// matches the convention in `tdw-domain` (`QuoteSnapshot::ts_ms`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Article {
    /// Headline / title of the article. Must be non-empty to pass [`is_valid`].
    pub title: String,
    /// Canonical URL of the article. Must be non-empty to pass [`is_valid`].
    pub url: String,
    /// Human-readable source name (e.g. `"Benzinga"`, `"Tiingo"`).
    pub source: String,
    /// Publication timestamp as Unix epoch milliseconds (UTC).
    pub published_ts_ms: i64,
    /// Short summary or teaser text, truncated to
    /// [`ComposePolicy::max_summary_chars`] Unicode scalar values during
    /// normalization. If truncated, an ellipsis (`…`) is appended.
    pub summary: String,
    /// Ticker symbols mentioned in the article (normalized to uppercase,
    /// deduplicated, sorted ascending after [`normalize`]).
    pub symbols: Vec<String>,
}

impl Article {
    /// Construct a new [`Article`].
    ///
    /// No validation is performed here; call [`is_valid`] to check the
    /// invariants required by the compose pipeline.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        url: impl Into<String>,
        source: impl Into<String>,
        published_ts_ms: i64,
        summary: impl Into<String>,
        symbols: Vec<String>,
    ) -> Self {
        Self {
            title: title.into(),
            url: url.into(),
            source: source.into(),
            published_ts_ms,
            summary: summary.into(),
            symbols,
        }
    }
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors returned by the news-compose pipeline.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NewsComposeError {
    /// The input article list was empty.
    #[error("article list is empty")]
    Empty,
    /// An article failed validation (non-empty title and URL required).
    #[error("invalid article: {0}")]
    Invalid(String),
}

// ---------------------------------------------------------------------------
// ComposePolicy
// ---------------------------------------------------------------------------

/// Tunable knobs for the news-aggregation pipeline.
///
/// Construct via [`Default`] and override individual fields as needed.
///
/// ```
/// use tdw_news_compose::ComposePolicy;
///
/// let policy = ComposePolicy { cap: 10, ..ComposePolicy::default() };
/// assert_eq!(policy.cap, 10);
/// assert_eq!(policy.max_summary_chars, 280);
/// assert!(policy.dedup);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposePolicy {
    /// Maximum number of articles to return from any compose operation.
    /// Defaults to `6`.
    pub cap: usize,
    /// Maximum number of Unicode scalar values kept in
    /// [`Article::summary`] after [`normalize`]. Truncation appends `…`.
    /// Defaults to `280`.
    pub max_summary_chars: usize,
    /// When `true` (default), duplicate URLs (or titles when the URL is
    /// empty) are removed during composition; only the first occurrence is
    /// kept.
    pub dedup: bool,
}

impl Default for ComposePolicy {
    fn default() -> Self {
        Self {
            cap: 6,
            max_summary_chars: 280,
            dedup: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Returns `true` when `article` satisfies the minimum structural invariants:
/// non-empty `title` **and** non-empty `url` (after trimming).
///
/// Articles that fail this predicate are silently dropped by [`compose`] and
/// [`compose_for_symbols`].
#[must_use]
pub fn is_valid(article: &Article) -> bool {
    !article.title.trim().is_empty() && !article.url.trim().is_empty()
}

// ---------------------------------------------------------------------------
// Normalize
// ---------------------------------------------------------------------------

/// Return a normalized copy of `raw`.
///
/// Normalization performs:
/// 1. Trim whitespace from all string scalar fields (`title`, `url`, `source`,
///    `summary`).
/// 2. Truncate `summary` to at most `max_summary_chars` Unicode scalar values.
///    Truncation always occurs on a Unicode scalar (char) boundary — never
///    mid-codepoint. When the summary is truncated, the `…` (U+2026 HORIZONTAL
///    ELLIPSIS) character is appended, so the returned summary contains at most
///    `max_summary_chars` chars followed by `…`.
/// 3. Uppercase each symbol, trim whitespace, drop empty entries, then
///    deduplicate (keeping first occurrence) and sort ascending.
#[must_use]
pub fn normalize(raw: Article, policy: &ComposePolicy) -> Article {
    let title = raw.title.trim().to_string();
    let url = raw.url.trim().to_string();
    let source = raw.source.trim().to_string();

    let trimmed_summary = raw.summary.trim();
    let summary = truncate_summary(trimmed_summary, policy.max_summary_chars);

    let symbols = normalize_symbols(raw.symbols);

    Article {
        title,
        url,
        source,
        published_ts_ms: raw.published_ts_ms,
        summary,
        symbols,
    }
}

/// Truncate `s` to at most `max_chars` Unicode scalar values.
///
/// If `s` is longer than `max_chars` chars, the returned string contains
/// exactly the first `max_chars` chars followed by `…` (U+2026). The split
/// always happens on a char boundary (guaranteed by iterating chars).
fn truncate_summary(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let mut out = String::with_capacity(s.len().min(max_chars * 4 + 3));
    let mut truncated = false;
    for (count, ch) in chars.by_ref().enumerate() {
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

/// Uppercase, trim, drop empties, dedup (keep first), sort ascending.
fn normalize_symbols(raw: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out: Vec<String> = raw
        .into_iter()
        .filter_map(|s| {
            let upper = s.trim().to_ascii_uppercase();
            if upper.is_empty() {
                None
            } else if seen.insert(upper.clone()) {
                Some(upper)
            } else {
                None
            }
        })
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Dedup
// ---------------------------------------------------------------------------

/// Remove duplicate articles from `articles`, keyed by normalized URL.
///
/// The first occurrence of each URL is kept; subsequent duplicates are
/// dropped. Order is otherwise preserved (stable). Articles with an empty
/// URL (which should have been filtered by [`is_valid`] first) fall back to
/// keying on the trimmed title.
#[must_use]
pub fn dedup(articles: Vec<Article>) -> Vec<Article> {
    let mut seen: HashSet<String> = HashSet::new();
    articles
        .into_iter()
        .filter(|a| {
            let key = if a.url.trim().is_empty() {
                a.title.trim().to_string()
            } else {
                a.url.trim().to_string()
            };
            seen.insert(key)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// compose (single-list)
// ---------------------------------------------------------------------------

/// Compose a single flat list of articles into a capped, sorted feed.
///
/// Pipeline: **validate** (drop invalid) → **normalize** → **dedup**
/// (when `policy.dedup`) → **sort newest-first** by `published_ts_ms` →
/// **cap** at `policy.cap`.
///
/// Returns an empty `Vec` when all articles are invalid or the input is empty.
#[must_use]
pub fn compose(articles: Vec<Article>, policy: &ComposePolicy) -> Vec<Article> {
    let mut out: Vec<Article> = articles
        .into_iter()
        .filter(is_valid)
        .map(|a| normalize(a, policy))
        .collect();

    if policy.dedup {
        out = dedup(out);
    }

    // Sort newest-first (descending published_ts_ms).
    out.sort_by_key(|a| Reverse(a.published_ts_ms));

    out.truncate(policy.cap);
    out
}

// ---------------------------------------------------------------------------
// compose_for_symbols (round-robin multi-symbol)
// ---------------------------------------------------------------------------

/// Compose a multi-symbol feed using round-robin distribution.
///
/// Symbols are iterated in [`BTreeMap`] key order (alphabetical). Articles
/// are drawn one-at-a-time per symbol in ascending round index until
/// `policy.cap` slots are filled or all articles are exhausted.
///
/// Within each symbol's list the pipeline is: **validate** → **normalize**
/// → per-round draw. After round-robin selection, **dedup** (when
/// `policy.dedup`) is applied across the whole merged set so a cross-symbol
/// duplicate is deduplicated (keeping the first one encountered in
/// round-robin order).
///
/// A symbol with fewer articles than its fair share simply yields its
/// remaining slots to subsequent rounds; if all symbols are exhausted
/// before `policy.cap` is reached the function returns however many
/// articles were available.
///
/// The returned slice is in round-robin emission order (symbol-spread),
/// **not** sorted by timestamp. Call [`compose`] if you want newest-first.
#[must_use]
pub fn compose_for_symbols(
    by_symbol: &BTreeMap<String, Vec<Article>>,
    policy: &ComposePolicy,
) -> Vec<Article> {
    // Pre-process each symbol's list: validate + normalize, drop invalids.
    let per_symbol: Vec<Vec<Article>> = by_symbol
        .values()
        .map(|articles| {
            articles
                .iter()
                .filter(|a| is_valid(a))
                .map(|a| normalize(a.clone(), policy))
                .collect()
        })
        .collect();

    let symbol_count = per_symbol.len();
    if symbol_count == 0 {
        return Vec::new();
    }

    // Round-robin: each pass through the symbols tries to take one article
    // from index `round` of each symbol's pre-processed list.
    let mut result: Vec<Article> = Vec::with_capacity(policy.cap);
    let mut round = 0usize;

    loop {
        if result.len() >= policy.cap {
            break;
        }
        let mut any_contributed = false;
        for symbol_list in &per_symbol {
            if result.len() >= policy.cap {
                break;
            }
            if let Some(article) = symbol_list.get(round) {
                result.push(article.clone());
                any_contributed = true;
            }
        }
        if !any_contributed {
            // All symbol lists are exhausted.
            break;
        }
        round += 1;
    }

    if policy.dedup {
        result = dedup(result);
        // Dedup can shrink below cap; we do not re-fill (deterministic output).
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn art(title: &str, url: &str, ts: i64, summary: &str, symbols: &[&str]) -> Article {
        Article::new(
            title,
            url,
            "TestSource",
            ts,
            summary,
            symbols.iter().map(|s| (*s).to_string()).collect(),
        )
    }

    // ── normalize ────────────────────────────────────────────────────────────

    #[test]
    fn normalize_trims_whitespace() {
        let policy = ComposePolicy::default();
        let a = art("  Hello  ", "  https://x.com  ", 0, "  body  ", &[]);
        let n = normalize(a, &policy);
        assert_eq!(n.title, "Hello");
        assert_eq!(n.url, "https://x.com");
        assert_eq!(n.summary, "body");
    }

    #[test]
    fn normalize_truncates_summary_at_char_limit() {
        let policy = ComposePolicy {
            max_summary_chars: 5,
            ..ComposePolicy::default()
        };
        let a = art("T", "U", 0, "123456789", &[]);
        let n = normalize(a, &policy);
        // First 5 chars + ellipsis
        assert_eq!(n.summary, "12345\u{2026}");
    }

    #[test]
    fn normalize_no_truncation_when_within_limit() {
        let policy = ComposePolicy {
            max_summary_chars: 10,
            ..ComposePolicy::default()
        };
        let a = art("T", "U", 0, "Hello", &[]);
        let n = normalize(a, &policy);
        assert_eq!(n.summary, "Hello");
    }

    #[test]
    fn normalize_truncates_multibyte_unicode_on_char_boundary() {
        // Each of these chars is 3 bytes in UTF-8 (CJK block).
        let long_cjk: String = "中文测试内容过长检验".to_string(); // 9 chars
        let policy = ComposePolicy {
            max_summary_chars: 4,
            ..ComposePolicy::default()
        };
        let a = art("T", "U", 0, &long_cjk, &[]);
        let n = normalize(a, &policy);
        // Should be valid UTF-8 and exactly 4 chars + ellipsis
        let chars: Vec<char> = n.summary.chars().collect();
        assert_eq!(chars.last(), Some(&'…'));
        assert_eq!(chars.len(), 5); // 4 content chars + ellipsis
        // Verify it's valid UTF-8 (would panic on bad boundary)
        assert!(std::str::from_utf8(n.summary.as_bytes()).is_ok());
    }

    #[test]
    fn normalize_uppercases_and_deduplicates_symbols() {
        let policy = ComposePolicy::default();
        let a = art("T", "U", 0, "", &["aapl", "AAPL", "msft", "MSFT", "goog"]);
        let n = normalize(a, &policy);
        assert_eq!(n.symbols, vec!["AAPL", "GOOG", "MSFT"]);
    }

    #[test]
    fn normalize_sorts_symbols_ascending() {
        let policy = ComposePolicy::default();
        let a = art("T", "U", 0, "", &["TSLA", "AAPL", "MSFT"]);
        let n = normalize(a, &policy);
        assert_eq!(n.symbols, vec!["AAPL", "MSFT", "TSLA"]);
    }

    // ── is_valid ─────────────────────────────────────────────────────────────

    #[test]
    fn is_valid_rejects_empty_title() {
        let a = art("", "https://example.com", 0, "", &[]);
        assert!(!is_valid(&a));
    }

    #[test]
    fn is_valid_rejects_empty_url() {
        let a = art("Title", "", 0, "", &[]);
        assert!(!is_valid(&a));
    }

    #[test]
    fn is_valid_rejects_whitespace_only_title() {
        let a = art("   ", "https://example.com", 0, "", &[]);
        assert!(!is_valid(&a));
    }

    #[test]
    fn is_valid_accepts_non_empty_title_and_url() {
        let a = art("Some Title", "https://example.com", 0, "", &[]);
        assert!(is_valid(&a));
    }

    // ── dedup ────────────────────────────────────────────────────────────────

    #[test]
    fn dedup_removes_same_url_keeps_first() {
        let articles = vec![
            art("First", "https://a.com", 100, "", &[]),
            art("Second", "https://b.com", 90, "", &[]),
            art("Duplicate of First", "https://a.com", 80, "", &[]),
        ];
        let out = dedup(articles);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].title, "First");
        assert_eq!(out[1].title, "Second");
    }

    #[test]
    fn dedup_preserves_order_of_unique_articles() {
        let articles = vec![
            art("C", "https://c.com", 3, "", &[]),
            art("A", "https://a.com", 1, "", &[]),
            art("B", "https://b.com", 2, "", &[]),
        ];
        let out = dedup(articles);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].title, "C");
        assert_eq!(out[1].title, "A");
        assert_eq!(out[2].title, "B");
    }

    // ── compose (single-list) ────────────────────────────────────────────────

    #[test]
    fn compose_sorts_newest_first_and_caps() {
        let policy = ComposePolicy {
            cap: 3,
            ..ComposePolicy::default()
        };
        let articles = vec![
            art("Old", "https://1.com", 100, "", &[]),
            art("Newest", "https://2.com", 300, "", &[]),
            art("Mid", "https://3.com", 200, "", &[]),
            art("Oldest", "https://4.com", 50, "", &[]),
        ];
        let out = compose(articles, &policy);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].title, "Newest");
        assert_eq!(out[1].title, "Mid");
        assert_eq!(out[2].title, "Old");
    }

    #[test]
    fn compose_drops_invalid_articles() {
        let policy = ComposePolicy::default();
        let articles = vec![
            art("", "https://no-title.com", 100, "", &[]),
            art("Valid", "https://valid.com", 200, "", &[]),
            art("No URL", "", 300, "", &[]),
        ];
        let out = compose(articles, &policy);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "Valid");
    }

    #[test]
    fn compose_deduplicates_when_enabled() {
        let policy = ComposePolicy {
            cap: 10,
            dedup: true,
            ..ComposePolicy::default()
        };
        let articles = vec![
            art("First", "https://dup.com", 200, "", &[]),
            art("Second", "https://dup.com", 100, "", &[]),
        ];
        let out = compose(articles, &policy);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn compose_deterministic_for_same_input() {
        let policy = ComposePolicy::default();
        let articles = vec![
            art("B", "https://b.com", 200, "", &[]),
            art("A", "https://a.com", 300, "", &[]),
            art("C", "https://c.com", 100, "", &[]),
        ];
        let out1 = compose(articles.clone(), &policy);
        let out2 = compose(articles, &policy);
        assert_eq!(out1, out2);
    }

    // ── compose_for_symbols ──────────────────────────────────────────────────

    #[test]
    fn compose_for_symbols_single_symbol_cap_limited() {
        let policy = ComposePolicy {
            cap: 2,
            ..ComposePolicy::default()
        };
        let mut by_symbol = BTreeMap::new();
        by_symbol.insert(
            "AAPL".to_string(),
            vec![
                art("A1", "https://1.com", 100, "", &[]),
                art("A2", "https://2.com", 200, "", &[]),
                art("A3", "https://3.com", 300, "", &[]),
            ],
        );
        let out = compose_for_symbols(&by_symbol, &policy);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn compose_for_symbols_two_symbols_even_split() {
        let policy = ComposePolicy {
            cap: 4,
            ..ComposePolicy::default()
        };
        let mut by_symbol = BTreeMap::new();
        by_symbol.insert(
            "AAPL".to_string(),
            vec![
                art("A1", "https://a1.com", 100, "", &[]),
                art("A2", "https://a2.com", 200, "", &[]),
            ],
        );
        by_symbol.insert(
            "MSFT".to_string(),
            vec![
                art("M1", "https://m1.com", 100, "", &[]),
                art("M2", "https://m2.com", 200, "", &[]),
            ],
        );
        let out = compose_for_symbols(&by_symbol, &policy);
        // Round-robin: AAPL[0], MSFT[0], AAPL[1], MSFT[1]
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].title, "A1");
        assert_eq!(out[1].title, "M1");
        assert_eq!(out[2].title, "A2");
        assert_eq!(out[3].title, "M2");
    }

    #[test]
    fn compose_for_symbols_three_symbols_uneven_short_list_yields_slots() {
        let policy = ComposePolicy {
            cap: 6,
            ..ComposePolicy::default()
        };
        let mut by_symbol = BTreeMap::new();
        // AAPL: 3 articles, GOOG: 1 article (short), MSFT: 3 articles
        by_symbol.insert(
            "AAPL".to_string(),
            vec![
                art("A1", "https://a1.com", 1, "", &[]),
                art("A2", "https://a2.com", 2, "", &[]),
                art("A3", "https://a3.com", 3, "", &[]),
            ],
        );
        by_symbol.insert(
            "GOOG".to_string(),
            vec![art("G1", "https://g1.com", 1, "", &[])],
        );
        by_symbol.insert(
            "MSFT".to_string(),
            vec![
                art("M1", "https://m1.com", 1, "", &[]),
                art("M2", "https://m2.com", 2, "", &[]),
                art("M3", "https://m3.com", 3, "", &[]),
            ],
        );
        let out = compose_for_symbols(&by_symbol, &policy);
        // BTreeMap key order: AAPL < GOOG < MSFT
        // Round 0: A1, G1, M1 → 3
        // Round 1: A2, (GOOG exhausted), M2 → 5
        // Round 2: A3 fills slot 6 (cap reached before MSFT[2] is drawn)
        assert_eq!(out.len(), 6);
        // AAPL contributes 3, GOOG contributes 1, MSFT contributes 2
        let aapl_count = out.iter().filter(|a| a.title.starts_with('A')).count();
        let msft_count = out.iter().filter(|a| a.title.starts_with('M')).count();
        let goog_count = out.iter().filter(|a| a.title.starts_with('G')).count();
        assert_eq!(aapl_count, 3);
        assert_eq!(msft_count, 2);
        assert_eq!(goog_count, 1); // only 1 available
    }

    #[test]
    fn compose_for_symbols_total_available_less_than_cap_returns_all() {
        let policy = ComposePolicy {
            cap: 20,
            ..ComposePolicy::default()
        };
        let mut by_symbol = BTreeMap::new();
        by_symbol.insert(
            "AAPL".to_string(),
            vec![
                art("A1", "https://a1.com", 1, "", &[]),
                art("A2", "https://a2.com", 2, "", &[]),
            ],
        );
        by_symbol.insert(
            "MSFT".to_string(),
            vec![art("M1", "https://m1.com", 1, "", &[])],
        );
        let out = compose_for_symbols(&by_symbol, &policy);
        // Total available = 3, cap = 20 → all returned
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn compose_for_symbols_each_symbol_contributes_when_it_has_articles() {
        let policy = ComposePolicy {
            cap: 6,
            ..ComposePolicy::default()
        };
        let mut by_symbol = BTreeMap::new();
        for sym in &["AAPL", "GOOG", "MSFT", "TSLA"] {
            by_symbol.insert(
                (*sym).to_string(),
                vec![
                    art(
                        &format!("{sym}-1"),
                        &format!("https://{sym}-1.com"),
                        1,
                        "",
                        &[],
                    ),
                    art(
                        &format!("{sym}-2"),
                        &format!("https://{sym}-2.com"),
                        2,
                        "",
                        &[],
                    ),
                ],
            );
        }
        let out = compose_for_symbols(&by_symbol, &policy);
        assert_eq!(out.len(), 6);
        // Round 0: AAPL-1, GOOG-1, MSFT-1, TSLA-1 (4 slots used)
        // Round 1: AAPL-2, GOOG-2 (2 more → cap=6)
        // Each of the 4 symbols should contribute at least 1
        for sym in &["AAPL", "GOOG", "MSFT", "TSLA"] {
            let contributed = out.iter().any(|a| a.title.starts_with(sym));
            assert!(contributed, "symbol {sym} should have contributed");
        }
    }

    #[test]
    fn compose_for_symbols_empty_map_returns_empty() {
        let policy = ComposePolicy::default();
        let by_symbol: BTreeMap<String, Vec<Article>> = BTreeMap::new();
        let out = compose_for_symbols(&by_symbol, &policy);
        assert!(out.is_empty());
    }

    #[test]
    fn compose_for_symbols_deterministic_output() {
        let policy = ComposePolicy {
            cap: 4,
            ..ComposePolicy::default()
        };
        let mut by_symbol = BTreeMap::new();
        by_symbol.insert(
            "AAPL".to_string(),
            vec![
                art("A1", "https://a1.com", 1, "", &[]),
                art("A2", "https://a2.com", 2, "", &[]),
            ],
        );
        by_symbol.insert(
            "MSFT".to_string(),
            vec![
                art("M1", "https://m1.com", 1, "", &[]),
                art("M2", "https://m2.com", 2, "", &[]),
            ],
        );
        let out1 = compose_for_symbols(&by_symbol, &policy);
        let out2 = compose_for_symbols(&by_symbol, &policy);
        assert_eq!(out1, out2);
    }
}
