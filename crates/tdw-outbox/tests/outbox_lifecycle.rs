#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-outbox lifecycle transitions.
// Authored offline. Verify with `cargo test --package tdw-outbox`.

use tdw_event::sample_event;
use tdw_outbox::{InMemoryOutbox, OutboxStatus};

#[test]
fn append_assigns_monotonic_sequences_starting_at_one() {
    let mut outbox = InMemoryOutbox::default();
    let s1 = outbox.append(sample_event("a"));
    let s2 = outbox.append(sample_event("b"));
    let s3 = outbox.append(sample_event("c"));
    assert_eq!(s1, 1);
    assert_eq!(s2, 2);
    assert_eq!(s3, 3);
}

#[test]
fn pending_after_filters_dispatched_records() {
    let mut outbox = InMemoryOutbox::default();
    let s1 = outbox.append(sample_event("a"));
    let s2 = outbox.append(sample_event("b"));
    let s3 = outbox.append(sample_event("c"));

    outbox.mark_dispatched(s2);

    let pending = outbox.pending_after(0);
    let sequences: Vec<u64> = pending.iter().map(|r| r.sequence).collect();
    assert_eq!(sequences, vec![s1, s3]);
    for record in &pending {
        assert_eq!(record.status, OutboxStatus::Pending);
    }
}

#[test]
fn pending_after_uses_strictly_greater_sequence_filter() {
    let mut outbox = InMemoryOutbox::default();
    let s1 = outbox.append(sample_event("a"));
    let s2 = outbox.append(sample_event("b"));
    let pending = outbox.pending_after(s1);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].sequence, s2);
}

#[test]
fn pending_after_with_high_sequence_returns_empty() {
    let mut outbox = InMemoryOutbox::default();
    outbox.append(sample_event("a"));
    let pending = outbox.pending_after(99);
    assert!(pending.is_empty());
}

#[test]
fn mark_dispatched_for_unknown_sequence_is_noop() {
    let mut outbox = InMemoryOutbox::default();
    outbox.append(sample_event("a"));
    outbox.mark_dispatched(999);
    let pending = outbox.pending_after(0);
    assert_eq!(
        pending.len(),
        1,
        "unknown dispatch must not touch existing records"
    );
}

#[test]
fn mark_dispatched_twice_is_idempotent() {
    let mut outbox = InMemoryOutbox::default();
    let s1 = outbox.append(sample_event("a"));
    outbox.mark_dispatched(s1);
    outbox.mark_dispatched(s1);
    assert!(outbox.pending_after(0).is_empty());
}

#[test]
fn empty_outbox_has_no_pending_records() {
    let outbox = InMemoryOutbox::default();
    assert!(outbox.pending_after(0).is_empty());
}

#[test]
fn outbox_status_round_trips() {
    for status in [OutboxStatus::Pending, OutboxStatus::Dispatched] {
        let json =
            serde_json::to_value(status).unwrap_or_else(|error| panic!("serialize: {error}"));
        let decoded: OutboxStatus =
            serde_json::from_value(json).unwrap_or_else(|error| panic!("deserialize: {error}"));
        assert_eq!(decoded, status);
    }
}
