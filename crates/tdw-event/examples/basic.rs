//! Offline `tdw-event` example: build and validate an event envelope, derive a
//! child event (causation + depth), and export the JSON-schema bundle.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p tdw-event --example tdw_event_basic
//! ```

#![forbid(unsafe_code)]

use serde_json::json;
use tdw_event::{EVENT_SCHEMA_NAMES, event_schema_bundle, sample_event};
use validator::Validate;

fn main() {
    // A sample envelope for the "mcp" entrypoint; event_type = "ingress.received".
    let event = sample_event("mcp");
    assert!(event.validate().is_ok(), "identity fields are non-empty");
    println!(
        "event {} from {}/{} (depth {})",
        event.event_id, event.origin.service, event.origin.entrypoint, event.depth
    );

    // A child event inherits identity, records causation, and increments depth.
    let child = event.child_event("hook.audit", json!({ "ok": true }));
    assert_eq!(child.causation_id.as_deref(), Some(event.event_id.as_str()));
    assert_eq!(child.depth, 1);
    assert_eq!(child.trace.trace_id, event.trace.trace_id); // same trace
    assert_eq!(child.trace.parent_span_id.as_deref(), Some("root"));
    println!(
        "child {} caused_by {:?} at depth {}",
        child.event_id, child.causation_id, child.depth
    );

    // The schema bundle exports all five named schemas.
    let bundle = event_schema_bundle();
    for name in EVENT_SCHEMA_NAMES {
        assert!(bundle.contains_key(name), "missing schema {name}");
    }
    println!("schema bundle exports: {:?}", EVENT_SCHEMA_NAMES);
}
