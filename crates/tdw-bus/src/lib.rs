#![forbid(unsafe_code)]

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_event::EventEnvelope;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BusEntry {
    pub sequence: u64,
    pub envelope: EventEnvelope<Value>,
}

#[derive(Clone, Debug)]
pub struct EventBus {
    capacity: usize,
    next_sequence: u64,
    events: VecDeque<BusEntry>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_sequence: 1,
            events: VecDeque::new(),
        }
    }

    pub fn publish(&mut self, envelope: EventEnvelope<Value>) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.events.push_back(BusEntry { sequence, envelope });
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
        sequence
    }

    pub fn read_from(&self, sequence: u64) -> Vec<BusEntry> {
        self.events
            .iter()
            .filter(|entry| entry.sequence >= sequence)
            .cloned()
            .collect()
    }

    pub fn lag_since(&self, last_seen_sequence: u64) -> u64 {
        self.next_sequence
            .saturating_sub(1)
            .saturating_sub(last_seen_sequence)
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_event::sample_event;

    #[test]
    fn publishes_reads_and_reports_lag_without_loss_within_capacity() {
        let mut bus = EventBus::new(4);
        let first = bus.publish(sample_event("service"));
        let second = bus.publish(sample_event("worker"));

        let entries = bus.read_from(first);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, first);
        assert_eq!(bus.lag_since(first), second - first);
    }
}
