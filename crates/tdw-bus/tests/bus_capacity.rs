#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for tdw-bus capacity, sequence monotonicity, and lag.
// Authored offline. Verify with `cargo test --package tdw-bus`.

use tdw_bus::EventBus;
use tdw_event::sample_event;

#[test]
fn new_with_zero_capacity_clamps_to_one() {
    let mut bus = EventBus::new(0);
    let first = bus.publish(sample_event("a"));
    let second = bus.publish(sample_event("b"));
    let entries = bus.read_from(0);
    assert_eq!(entries.len(), 1, "capacity 0 must clamp to 1");
    assert_eq!(entries[0].sequence, second);
    assert!(second > first);
}

#[test]
fn capacity_evicts_oldest_when_exceeded() {
    let mut bus = EventBus::new(2);
    bus.publish(sample_event("a"));
    bus.publish(sample_event("b"));
    bus.publish(sample_event("c"));

    let entries = bus.read_from(0);
    assert_eq!(entries.len(), 2);
    // oldest "a" (sequence 1) should be gone; b (2) and c (3) remain
    assert_eq!(entries[0].sequence, 2);
    assert_eq!(entries[1].sequence, 3);
}

#[test]
fn sequences_are_monotonic_starting_at_one() {
    let mut bus = EventBus::new(8);
    let s1 = bus.publish(sample_event("a"));
    let s2 = bus.publish(sample_event("b"));
    let s3 = bus.publish(sample_event("c"));
    assert_eq!(s1, 1);
    assert_eq!(s2, 2);
    assert_eq!(s3, 3);
}

#[test]
fn read_from_future_sequence_returns_empty() {
    let mut bus = EventBus::new(8);
    bus.publish(sample_event("a"));
    let entries = bus.read_from(99);
    assert!(entries.is_empty());
}

#[test]
fn read_from_zero_returns_all_visible_entries() {
    let mut bus = EventBus::new(8);
    bus.publish(sample_event("a"));
    bus.publish(sample_event("b"));
    let entries = bus.read_from(0);
    assert_eq!(entries.len(), 2);
}

#[test]
fn lag_since_zero_with_empty_bus_is_zero() {
    let bus = EventBus::new(8);
    assert_eq!(bus.lag_since(0), 0);
}

#[test]
fn lag_since_returns_distance_to_latest_sequence() {
    let mut bus = EventBus::new(8);
    bus.publish(sample_event("a")); // sequence 1
    bus.publish(sample_event("b")); // sequence 2
    bus.publish(sample_event("c")); // sequence 3
    assert_eq!(bus.lag_since(0), 3);
    assert_eq!(bus.lag_since(1), 2);
    assert_eq!(bus.lag_since(3), 0);
}

#[test]
fn lag_since_saturates_when_caller_overshoots() {
    let mut bus = EventBus::new(8);
    bus.publish(sample_event("a"));
    assert_eq!(bus.lag_since(99), 0, "saturating_sub must clamp at 0");
}

#[test]
fn default_bus_has_high_capacity() {
    let mut bus = EventBus::default();
    for _ in 0..100 {
        bus.publish(sample_event("x"));
    }
    let entries = bus.read_from(0);
    assert_eq!(
        entries.len(),
        100,
        "default capacity should comfortably hold 100"
    );
}
