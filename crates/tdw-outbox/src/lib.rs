#![forbid(unsafe_code)]

#[cfg(feature = "postgres")]
pub mod pg_outbox;

#[cfg(feature = "postgres")]
pub use pg_outbox::PgOutboxStore;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_event::EventEnvelope;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutboxStatus {
    Pending,
    Dispatched,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutboxRecord {
    pub sequence: u64,
    pub envelope: EventEnvelope<Value>,
    pub status: OutboxStatus,
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryOutbox {
    next_sequence: u64,
    records: Vec<OutboxRecord>,
}

impl InMemoryOutbox {
    pub fn append(&mut self, envelope: EventEnvelope<Value>) -> u64 {
        let sequence = if self.next_sequence == 0 {
            1
        } else {
            self.next_sequence
        };
        self.next_sequence = sequence + 1;
        self.records.push(OutboxRecord {
            sequence,
            envelope,
            status: OutboxStatus::Pending,
        });
        sequence
    }

    pub fn mark_dispatched(&mut self, sequence: u64) -> bool {
        for record in &mut self.records {
            if record.sequence == sequence {
                record.status = OutboxStatus::Dispatched;
                return true;
            }
        }
        false
    }

    pub fn pending_after(&self, sequence: u64) -> Vec<OutboxRecord> {
        self.records
            .iter()
            .filter(|record| record.sequence > sequence && record.status == OutboxStatus::Pending)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_event::sample_event;

    #[test]
    fn recovers_pending_events_after_dispatch_boundary() {
        let mut outbox = InMemoryOutbox::default();
        let first = outbox.append(sample_event("service"));
        let second = outbox.append(sample_event("worker"));
        assert!(outbox.mark_dispatched(first));
        assert!(!outbox.mark_dispatched(999));

        let pending = outbox.pending_after(0);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sequence, second);
    }
}
