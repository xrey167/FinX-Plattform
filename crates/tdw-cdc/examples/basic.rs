//! Offline `tdw-cdc` example: append events to an in-memory outbox, project them
//! into a CDC change stream, and tail from an offset cursor.
//!
//! Run with:
//!
//! ```text
//! cargo run -p tdw-cdc --example basic
//! ```

use tdw_cdc::CdcStream;
use tdw_event::sample_event;
use tdw_outbox::InMemoryOutbox;

fn main() {
    // Two domain writes land in the transactional outbox.
    let mut outbox = InMemoryOutbox::default();
    outbox.append(sample_event("service"));
    outbox.append(sample_event("worker"));

    // Meaningful operation: project the outbox into an ordered change stream.
    let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
    println!("captured {} change(s):", cdc.records.len());
    for record in &cdc.records {
        println!("  offset={} type={}", record.offset, record.event_type);
    }

    // A consumer that already processed offset 1 tails the rest.
    let tail = cdc.after(1);
    println!("after offset 1 -> {} record(s)", tail.len());
    println!("first new offset: {}", tail[0].offset);
}
