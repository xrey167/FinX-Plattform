#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-cdc: outbox-to-cdc projection, offset filtering,
// payload fidelity.
// Authored offline. Verify with `cargo test --package tdw-cdc`.

use tdw_cdc::CdcStream;
use tdw_event::sample_event;
use tdw_outbox::InMemoryOutbox;

#[test]
fn empty_outbox_produces_empty_cdc_stream() {
    let outbox = InMemoryOutbox::default();
    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    assert!(cdc.records.is_empty());
    assert!(cdc.after(0).is_empty());
}

#[test]
fn from_outbox_preserves_event_id_and_type_per_record() {
    let mut outbox = InMemoryOutbox::default();
    outbox.append(sample_event("a"));
    outbox.append(sample_event("b"));

    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    assert_eq!(cdc.records.len(), 2);
    for record in &cdc.records {
        // sample_event uses "ingress.received" as event_type
        assert_eq!(record.event_type, "ingress.received");
        assert!(!record.event_id.is_empty());
    }
}

#[test]
fn from_outbox_assigns_offset_from_outbox_sequence() {
    let mut outbox = InMemoryOutbox::default();
    let s1 = outbox.append(sample_event("a"));
    let s2 = outbox.append(sample_event("b"));
    let s3 = outbox.append(sample_event("c"));

    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    assert_eq!(cdc.records[0].offset, s1);
    assert_eq!(cdc.records[1].offset, s2);
    assert_eq!(cdc.records[2].offset, s3);
}

#[test]
fn after_zero_returns_all_records() {
    let mut outbox = InMemoryOutbox::default();
    outbox.append(sample_event("a"));
    outbox.append(sample_event("b"));
    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    assert_eq!(cdc.after(0).len(), 2);
}

#[test]
fn after_uses_strictly_greater_offset_filter() {
    let mut outbox = InMemoryOutbox::default();
    outbox.append(sample_event("a"));
    outbox.append(sample_event("b"));
    outbox.append(sample_event("c"));
    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    let after_one = cdc.after(1);
    assert_eq!(after_one.len(), 2);
    assert_eq!(after_one[0].offset, 2);
    assert_eq!(after_one[1].offset, 3);
}

#[test]
fn after_with_offset_past_end_is_empty() {
    let mut outbox = InMemoryOutbox::default();
    outbox.append(sample_event("a"));
    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    assert!(cdc.after(99).is_empty());
}

#[test]
fn from_outbox_skips_dispatched_records_when_caller_filters_first() {
    let mut outbox = InMemoryOutbox::default();
    let s1 = outbox.append(sample_event("a"));
    outbox.append(sample_event("b"));
    outbox.mark_dispatched(s1);
    // pending_after already filters; CDC just projects whatever is given.
    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    assert_eq!(cdc.records.len(), 1);
    assert_eq!(cdc.records[0].offset, 2);
}

#[test]
fn cdc_records_round_trip_via_serde() {
    let mut outbox = InMemoryOutbox::default();
    outbox.append(sample_event("a"));
    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    let json = serde_json::to_string(&cdc).unwrap_or_else(|e| panic!("serialize: {e}"));
    let decoded: CdcStream =
        serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
    assert_eq!(decoded, cdc);
}
