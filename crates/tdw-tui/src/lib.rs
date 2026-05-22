#![forbid(unsafe_code)]

use ratatui::text::Line;
use tdw_protocol::EventMsg;

pub fn event_lines(events: &[EventMsg]) -> Vec<Line<'static>> {
    events.iter().map(event_line).collect()
}

pub fn event_line(event: &EventMsg) -> Line<'static> {
    match event {
        EventMsg::Started { .. } => Line::from("started"),
        EventMsg::Progress {
            stage, fraction, ..
        } => Line::from(format!("progress {stage} {fraction:.2}")),
        EventMsg::ApprovalRequested { action, .. } => Line::from(format!("approval {action}")),
        EventMsg::ToolCallRequested { tool_name, .. } => {
            Line::from(format!("tool requested {tool_name}"))
        }
        EventMsg::ToolCallCompleted { call_id, .. } => {
            Line::from(format!("tool completed {}", call_id.as_str()))
        }
        EventMsg::OutputChunk { stream, bytes, .. } => Line::from(format!("{stream:?} {bytes}")),
        EventMsg::DomainEvent { event_type, .. } => Line::from(format!("domain {event_type}")),
        EventMsg::Completed { summary, .. } => Line::from(format!(
            "completed {}",
            summary.as_deref().unwrap_or_default()
        )),
        EventMsg::Failed { error, .. } => Line::from(format!("failed {error}")),
        EventMsg::Cancelled { reason, .. } => Line::from(format!(
            "cancelled {}",
            reason.as_deref().unwrap_or_default()
        )),
    }
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
}
