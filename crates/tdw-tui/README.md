# tdw-tui

Terminal-UI rendering helpers for TDW. Converts `tdw_protocol::EventMsg` values
into `ratatui` `Line`s for display in a terminal client, with control-character
sanitization and length capping so untrusted event text cannot corrupt the
terminal.

This crate is presentation-only: `#![forbid(unsafe_code)]`, no event loop, no
terminal driver, no I/O. It is a pure `EventMsg -> Line` mapping that a TUI host
(or `tdw-service-api`'s client-event sample) calls to format daemon events.

## Binaries produced

None. Library crate.

## Feature flags

None.

## Key environment variables

None. See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the
platform's `TDW_*` reference.

## Quickstart (library)

Render a slice of daemon events as terminal lines:

```rust
use tdw_protocol::{EventMsg, OpId};
use tdw_tui::event_lines;

let events = vec![
    EventMsg::Started { op_id: OpId::generated() },
    EventMsg::Completed { op_id: OpId::generated(), summary: Some("done".into()), result: None },
];
let lines = event_lines(&events);
assert_eq!(lines[0].spans[0].content, "started");
assert_eq!(lines[1].spans[0].content, "completed done");
```

`event_line(&EventMsg)` renders a single event; `sanitize_event_text(&str)`
strips control characters and caps length (used internally for every dynamic
field).

See [`examples/basic.rs`](examples/basic.rs):
`cargo run -p tdw-tui --example tdw_tui_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — the event-to-line mapping and sanitizer.
- `tdw-protocol` — the `EventMsg` source vocabulary.
