#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use tdw_protocol::{EventMsg, OpEnvelope};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaemonTransport {
    Uds,
    HttpSse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonEndpoint {
    pub transport: DaemonTransport,
    pub address: String,
}

#[derive(Clone)]
pub struct SubmissionHandle {
    sender: mpsc::UnboundedSender<OpEnvelope>,
}

#[derive(Debug)]
pub struct SubmissionError {
    envelope: Box<OpEnvelope>,
}

impl SubmissionError {
    pub fn into_envelope(self) -> OpEnvelope {
        *self.envelope
    }
}

impl SubmissionHandle {
    pub fn submit(&self, envelope: OpEnvelope) -> std::result::Result<(), SubmissionError> {
        self.sender.send(envelope).map_err(|error| SubmissionError {
            envelope: Box::new(error.0),
        })
    }
}

pub struct AgentLoop {
    submissions: mpsc::UnboundedReceiver<OpEnvelope>,
    events: mpsc::UnboundedSender<EventMsg>,
}

impl AgentLoop {
    pub async fn run_once(&mut self) -> Option<EventMsg> {
        tokio::select! {
            Some(envelope) = self.submissions.recv() => {
                let event = EventMsg::Started { op_id: envelope.op_id };
                let _ = self.events.send(event.clone());
                Some(event)
            }
            else => None,
        }
    }
}

pub fn channel() -> (
    SubmissionHandle,
    mpsc::UnboundedReceiver<EventMsg>,
    AgentLoop,
) {
    let (submission_tx, submission_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    (
        SubmissionHandle {
            sender: submission_tx,
        },
        event_rx,
        AgentLoop {
            submissions: submission_rx,
            events: event_tx,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, Op, SessionId};

    #[tokio::test]
    async fn agent_loop_emits_started_event_from_submission_queue() {
        let (handle, mut events, mut loop_runner) = channel();
        let envelope = OpEnvelope::new(
            SessionId::new("session-1").expect("session id"),
            1,
            ActorRef {
                actor_id: "user".to_string(),
                kind: ActorKind::User,
                tenant_id: None,
            },
            Op::Shutdown,
        );

        handle.submit(envelope).expect("submission accepted");
        let event = loop_runner.run_once().await.expect("event emitted");

        assert!(matches!(event, EventMsg::Started { .. }));
        assert!(matches!(
            events.recv().await,
            Some(EventMsg::Started { .. })
        ));
    }
}
