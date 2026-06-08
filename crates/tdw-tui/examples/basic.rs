//! Offline, no-network example for `tdw-tui`.
//!
//! Renders a batch of `tdw_protocol::EventMsg` values into ratatui `Line`s and
//! prints their text, then shows that control characters in event text are
//! sanitized. No terminal driver, no I/O beyond stdout.
//!
//! Run with: `cargo run -p tdw-tui --example tdw_tui_basic`

use tdw_protocol::{EventMsg, OpId};
use tdw_tui::{event_line, event_lines, sanitize_event_text};

fn main() {
    let events = vec![
        EventMsg::Started {
            op_id: OpId::generated(),
        },
        EventMsg::Progress {
            op_id: OpId::generated(),
            stage: "dispatch".to_string(),
            fraction: 0.5,
            message: None,
        },
        EventMsg::Completed {
            op_id: OpId::generated(),
            summary: Some("done".to_string()),
            result: None,
        },
    ];

    // Render the batch and print each line's text content.
    for line in event_lines(&events) {
        // A `Line` is a sequence of spans; these renders use a single span.
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        println!("line: {text}");
    }

    // Control characters in event text are replaced with spaces so a hostile
    // payload cannot inject terminal escape sequences.
    let failed = event_line(&EventMsg::Failed {
        op_id: OpId::generated(),
        error: "bad\nstatus\u{0007}".to_string(),
    });
    let rendered: String = failed.spans.iter().map(|s| s.content.as_ref()).collect();
    println!("sanitized failure line: {rendered:?}");

    // The sanitizer is also callable directly.
    println!("sanitize sample: {:?}", sanitize_event_text("a\tb\rc"));
}
