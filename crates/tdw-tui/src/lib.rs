#![forbid(unsafe_code)]

use ratatui::text::Line;
use tdw_protocol::EventMsg;

const MAX_EVENT_TEXT_LEN: usize = 160;

pub fn event_lines(events: &[EventMsg]) -> Vec<Line<'static>> {
    events.iter().map(event_line).collect()
}

pub fn event_line(event: &EventMsg) -> Line<'static> {
    match event {
        EventMsg::Started { .. } => Line::from("started"),
        EventMsg::Progress {
            stage, fraction, ..
        } => Line::from(format!(
            "progress {} {fraction:.2}",
            sanitize_event_text(stage)
        )),
        EventMsg::ApprovalRequested { action, .. } => {
            Line::from(format!("approval {}", sanitize_event_text(action)))
        }
        EventMsg::ToolCallRequested { tool_name, .. } => {
            Line::from(format!("tool requested {}", sanitize_event_text(tool_name)))
        }
        EventMsg::ToolCallCompleted { call_id, .. } => {
            Line::from(format!("tool completed {}", call_id.as_str()))
        }
        EventMsg::OutputChunk { stream, bytes, .. } => {
            Line::from(format!("{stream:?} {}", sanitize_event_text(bytes)))
        }
        EventMsg::DomainEvent { event_type, .. } => {
            Line::from(format!("domain {}", sanitize_event_text(event_type)))
        }
        EventMsg::Completed { summary, .. } => Line::from(format!(
            "completed {}",
            sanitize_event_text(summary.as_deref().unwrap_or_default())
        )),
        EventMsg::Failed { error, .. } => {
            Line::from(format!("failed {}", sanitize_event_text(error)))
        }
        EventMsg::Cancelled { reason, .. } => Line::from(format!(
            "cancelled {}",
            sanitize_event_text(reason.as_deref().unwrap_or_default())
        )),
    }
}

pub fn sanitize_event_text(value: &str) -> String {
    let mut sanitized = String::new();
    for ch in value.chars().take(MAX_EVENT_TEXT_LEN) {
        if ch.is_control() {
            sanitized.push(' ');
        } else {
            sanitized.push(ch);
        }
    }
    if value.chars().count() > MAX_EVENT_TEXT_LEN {
        sanitized.push_str("...");
    }
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{EventMsg, OpId};

    #[test]
    fn converts_protocol_events_to_ratatui_lines() {
        let events = vec![
            EventMsg::Started {
                op_id: OpId::generated(),
            },
            EventMsg::Completed {
                op_id: OpId::generated(),
                summary: Some("done".to_string()),
                result: None,
            },
        ];
        let lines = event_lines(&events);

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "started");
        assert_eq!(lines[1].spans[0].content, "completed done");
    }

    #[test]
    fn sanitizes_event_text_for_terminal_output() {
        let line = event_line(&EventMsg::Failed {
            op_id: OpId::generated(),
            error: "bad\nstatus\u{0007}".to_string(),
        });

        assert_eq!(line.spans[0].content, "failed bad status ");
    }
}
