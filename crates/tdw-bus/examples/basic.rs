//! Offline `EventBus` round-trip: publish events, replay from a cursor, and
//! observe retention eviction on the bounded ring. No network, no docker.
//!
//! Run with: `cargo run -p tdw-bus --example tdw-bus-basic`

use tdw_bus::EventBus;
use tdw_event::sample_event;

fn main() {
    // Bounded ring with capacity 2 to show retention eviction.
    let mut bus = EventBus::new(2);

    let first = bus.publish(sample_event("service"));
    let second = bus.publish(sample_event("worker"));
    let third = bus.publish(sample_event("mcp")); // evicts `first`

    // Replay everything still retained from the start.
    let retained: Vec<u64> = bus.read_from(first).iter().map(|e| e.sequence).collect();
    assert_eq!(retained, vec![second, third]);

    // The first cursor now points before the retained window.
    assert!(bus.has_retention_gap(first));
    assert!(!bus.has_retention_gap(second));

    println!(
        "bus ok: published {first}/{second}/{third}, retained = {retained:?}, gap_at({first}) = {}",
        bus.has_retention_gap(first)
    );
}
