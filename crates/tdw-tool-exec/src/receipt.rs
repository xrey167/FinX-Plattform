//! Opt-in, append-only, hash-chained tool-receipt log.
//!
//! When recording is enabled on a [`crate::ToolExecutor`], every *successful*
//! `execute(...)` appends one [`ToolReceipt`] whose `this_hash` chains over the
//! previous receipt's hash. [`ReceiptLog::verify`] walks the chain and reports the
//! first inconsistency, so post-hoc tampering (a reordered, mutated, or re-linked
//! receipt) is detectable.
//!
//! # Integrity, not cryptography
//!
//! The chain uses the standard-library [`std::collections::hash_map::DefaultHasher`]
//! (`SipHash` 1-3) over a deterministic byte encoding. This makes the log
//! **tamper-EVIDENT** against accidental corruption or a casual edit — it is **NOT**
//! cryptographic. `SipHash` is not second-preimage/collision resistant against a
//! motivated attacker who controls the receipts and can grind a `u64` collision.
//! Treat this as an in-process audit trail, not a security boundary. A future
//! upgrade to a real digest behind an opt-in feature is a clean follow-up.
//!
//! No new dependency is introduced: hashing is std-only and the canonical encoding
//! reuses the already-present `serde_json::Value`.
//!
//! # Append-only and thread-safety
//!
//! The log is append-only: there is no public removal/mutation API (no `pop`,
//! `clear`, or `&mut` accessor). Appends flow only through the owning
//! [`crate::ToolExecutor`], which preserves the chain invariant.
//!
//! Recording uses interior mutability ([`std::cell::RefCell`]) so the executor's
//! `execute(&self, ...)` signature is unchanged. As a result a receipts-enabled
//! [`crate::ToolExecutor`] is **not** `Sync`: it must not be shared across threads
//! for concurrent `execute`. Today's daemon/worker/MCP callers use the executor
//! per-call, not shared, so this is not a regression.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use serde_json::Value;

use crate::ToolOutcome;

/// Sentinel `prev_hash` for the first link (seq 0) of a chain, and the value
/// [`ReceiptLog::last_hash`] returns for an empty log.
pub const GENESIS_PREV_HASH: u64 = 0;

/// One link in the hash chain.
///
/// `this_hash == chain_hash(prev_hash, seq, tool, args_hash, result_hash)`. The
/// hashes are std `DefaultHasher` (`SipHash`) values — tamper-evident, not
/// cryptographic (see the [module docs](self)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolReceipt {
    /// 0-based monotonic index of this receipt within the chain.
    pub seq: u64,
    /// The resolved tool name passed to `execute()`.
    pub tool: String,
    /// Hash of the canonical args bytes.
    pub args_hash: u64,
    /// Hash of the canonical `ToolOutcome.structured` bytes.
    pub result_hash: u64,
    /// `this_hash` of the receipt at `seq - 1`, or [`GENESIS_PREV_HASH`] for `seq 0`.
    pub prev_hash: u64,
    /// Chain hash over `(prev_hash, seq, tool, args_hash, result_hash)`.
    pub this_hash: u64,
}

/// Append-only, in-memory hash-chained receipt log.
///
/// Uses interior mutability so the owning executor's `execute(&self, ...)` stays
/// `&self`; consequently the log is **not** `Sync` (see the [module docs](self)).
#[derive(Debug, Default)]
pub struct ReceiptLog {
    receipts: RefCell<Vec<ToolReceipt>>,
}

/// Outcome of verifying a chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChainStatus {
    /// Empty or fully consistent: every `seq` is contiguous, every `prev_hash`
    /// links, and every `this_hash` recomputes.
    Valid,
    /// First detected break, with the offending `seq` and which invariant failed.
    Broken {
        /// The `seq` index at which the chain first failed to verify.
        seq: u64,
        /// Which invariant the receipt at `seq` violated.
        reason: ChainBreak,
    },
}

/// The specific chain invariant a [`ChainStatus::Broken`] receipt violated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainBreak {
    /// `seq` was not equal to its expected contiguous index.
    Seq,
    /// `prev_hash` did not equal the previous receipt's `this_hash` (or
    /// [`GENESIS_PREV_HASH`] for `seq 0`).
    PrevHash,
    /// The recomputed `this_hash` did not equal the stored `this_hash`.
    ThisHash,
}

impl ReceiptLog {
    /// A new, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of receipts recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.receipts.borrow().len()
    }

    /// Whether the log has no receipts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.receipts.borrow().is_empty()
    }

    /// A snapshot copy of the recorded receipts (cheap to clone; receipts are small).
    ///
    /// Returns an owned `Vec` rather than a borrowed slice because the receipts live
    /// behind a [`RefCell`]; pass the result to [`ReceiptLog::verify`] to audit it.
    #[must_use]
    pub fn snapshot(&self) -> Vec<ToolReceipt> {
        self.receipts.borrow().clone()
    }

    /// The tail receipt's `this_hash`, or [`GENESIS_PREV_HASH`] if the log is empty.
    #[must_use]
    pub fn last_hash(&self) -> u64 {
        self.receipts
            .borrow()
            .last()
            .map_or(GENESIS_PREV_HASH, |receipt| receipt.this_hash)
    }

    /// Append a receipt for a successful execution. Crate-private: only
    /// [`crate::ToolExecutor`] may append, preserving the append-only invariant.
    ///
    /// Recording is infallible (in-memory push; hashing cannot fail). Returns the
    /// `seq` of the newly appended receipt.
    pub(crate) fn append(&self, tool: &str, args: &Value, outcome: &ToolOutcome) -> u64 {
        let mut receipts = self.receipts.borrow_mut();
        let seq = receipts.len() as u64;
        let prev_hash = receipts
            .last()
            .map_or(GENESIS_PREV_HASH, |receipt| receipt.this_hash);
        let args_hash = hash_value(args);
        let result_hash = hash_value(&outcome.structured);
        let this_hash = chain_hash(prev_hash, seq, tool, args_hash, result_hash);
        receipts.push(ToolReceipt {
            seq,
            tool: tool.to_string(),
            args_hash,
            result_hash,
            prev_hash,
            this_hash,
        });
        seq
    }

    /// Walk `receipts` and return [`ChainStatus::Valid`] or the first
    /// [`ChainStatus::Broken`].
    ///
    /// Static (takes a slice) so it can audit externally-supplied chains — e.g. a
    /// chain captured via [`ReceiptLog::snapshot`], persisted, and re-loaded. An
    /// empty slice is [`ChainStatus::Valid`].
    #[must_use]
    pub fn verify(receipts: &[ToolReceipt]) -> ChainStatus {
        let mut prev_hash = GENESIS_PREV_HASH;
        for (index, receipt) in receipts.iter().enumerate() {
            let expected_seq = index as u64;
            if receipt.seq != expected_seq {
                return ChainStatus::Broken {
                    seq: expected_seq,
                    reason: ChainBreak::Seq,
                };
            }
            if receipt.prev_hash != prev_hash {
                return ChainStatus::Broken {
                    seq: receipt.seq,
                    reason: ChainBreak::PrevHash,
                };
            }
            let recomputed = chain_hash(
                receipt.prev_hash,
                receipt.seq,
                &receipt.tool,
                receipt.args_hash,
                receipt.result_hash,
            );
            if recomputed != receipt.this_hash {
                return ChainStatus::Broken {
                    seq: receipt.seq,
                    reason: ChainBreak::ThisHash,
                };
            }
            prev_hash = receipt.this_hash;
        }
        ChainStatus::Valid
    }
}

/// Discriminant bytes prefixing each node of the canonical encoding so structurally
/// distinct values cannot collide (`["a","b"]` must not encode like `["ab"]`).
mod tag {
    pub const NULL: u8 = 0;
    pub const BOOL: u8 = 1;
    pub const NUMBER: u8 = 2;
    pub const STRING: u8 = 3;
    pub const ARRAY: u8 = 4;
    pub const OBJECT: u8 = 5;
}

/// Canonicalize a JSON value into a deterministic, order-independent byte encoding.
///
/// Each node is written as a discriminant tag, then a length-prefixed payload, so
/// that nesting and boundaries are unambiguous. Object keys are sorted so two
/// objects with the same entries hash equal regardless of insertion order. Numbers
/// use their `serde_json` string repr (stable; avoids float bit-twiddling).
fn to_canonical_bytes(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write_canonical(value, &mut out);
    out
}

fn write_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&(len as u64).to_le_bytes());
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    write_len(out, bytes.len());
    out.extend_from_slice(bytes);
}

fn write_canonical(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.push(tag::NULL),
        Value::Bool(b) => {
            out.push(tag::BOOL);
            out.push(u8::from(*b));
        }
        Value::Number(n) => {
            out.push(tag::NUMBER);
            write_str(out, &n.to_string());
        }
        Value::String(s) => {
            out.push(tag::STRING);
            write_str(out, s);
        }
        Value::Array(items) => {
            out.push(tag::ARRAY);
            write_len(out, items.len());
            for item in items {
                write_canonical(item, out);
            }
        }
        Value::Object(map) => {
            out.push(tag::OBJECT);
            write_len(out, map.len());
            // Sort keys so insertion order does not affect the encoding.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            for key in keys {
                write_str(out, key);
                // Present-key guaranteed by iterating the map's own keys.
                if let Some(child) = map.get(key) {
                    write_canonical(child, out);
                }
            }
        }
    }
}

/// Hash a JSON value via its canonical encoding.
fn hash_value(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    // `to_canonical_bytes` is consumed directly by `Hash::hash`; inlining avoids a
    // clippy::collection_is_never_read false positive (the lint doesn't count
    // `Hash::hash` as reading the Vec, though it does).
    to_canonical_bytes(value).hash(&mut hasher);
    hasher.finish()
}

/// Chain hash over the receipt fields, including the previous link.
///
/// Implements `this_hash = H(prev_hash || seq || tool || args_hash || result_hash)`.
/// The std `Hash` impls for the tuple and `str` length-prefix their bytes, so field
/// concatenation is unambiguous.
fn chain_hash(prev: u64, seq: u64, tool: &str, args_hash: u64, result_hash: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    (prev, seq, tool, args_hash, result_hash).hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn chain_hash_is_deterministic() {
        let a = chain_hash(7, 2, "tool.name", 11, 22);
        let b = chain_hash(7, 2, "tool.name", 11, 22);
        assert_eq!(a, b);
    }

    #[test]
    fn chain_hash_is_field_sensitive() {
        let base = chain_hash(1, 0, "t", 2, 3);
        assert_ne!(base, chain_hash(9, 0, "t", 2, 3), "prev matters");
        assert_ne!(base, chain_hash(1, 1, "t", 2, 3), "seq matters");
        assert_ne!(base, chain_hash(1, 0, "u", 2, 3), "tool matters");
        assert_ne!(base, chain_hash(1, 0, "t", 8, 3), "args_hash matters");
        assert_ne!(base, chain_hash(1, 0, "t", 2, 8), "result_hash matters");
    }

    #[test]
    fn hash_value_is_stable_for_primitives() {
        assert_eq!(hash_value(&json!(null)), hash_value(&json!(null)));
        assert_eq!(hash_value(&json!(true)), hash_value(&json!(true)));
        assert_eq!(hash_value(&json!(42)), hash_value(&json!(42)));
        assert_eq!(hash_value(&json!("hi")), hash_value(&json!("hi")));
    }

    #[test]
    fn hash_value_distinguishes_structures() {
        // ["a","b"] must not collide with ["ab"] thanks to length prefixes.
        assert_ne!(hash_value(&json!(["a", "b"])), hash_value(&json!(["ab"])));
        // Distinct primitives differ.
        assert_ne!(hash_value(&json!(1)), hash_value(&json!("1")));
        assert_ne!(hash_value(&json!(true)), hash_value(&json!(1)));
    }

    #[test]
    fn hash_value_is_object_key_order_independent() {
        let a = json!({ "x": 1, "y": 2 });
        let b = json!({ "y": 2, "x": 1 });
        assert_eq!(hash_value(&a), hash_value(&b));
    }
}
