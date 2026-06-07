//! Offline `InMemoryOutbox` round-trip: append two events, dispatch one, and
//! recover the still-pending one. No network, no docker — the in-memory outbox
//! is the always-available default.
//!
//! Run with: `cargo run -p tdw-outbox --example tdw-outbox-basic`

use tdw_event::sample_event;
use tdw_outbox::InMemoryOutbox;

fn main() {
    let mut outbox = InMemoryOutbox::default();

    let first = outbox.append(sample_event("service"));
    let second = outbox.append(sample_event("worker"));

    // Relay the first event; the second stays pending.
    outbox.mark_dispatched(first);

    // A relay loop recovers everything still pending after its last-seen cursor.
    let pending = outbox.pending_after(0);

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].sequence, second);
    println!(
        "outbox ok: appended {first} + {second}, dispatched {first}, recovered pending = [{}]",
        pending[0].sequence
    );
}
