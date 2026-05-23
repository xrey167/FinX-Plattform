#![forbid(unsafe_code)]

#[cfg(feature = "postgres")]
pub mod pg_bus;

#[cfg(feature = "postgres")]
pub use pg_bus::PgEventBus;

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_event::EventEnvelope;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BusEntry {
    pub sequence: u64,
    pub envelope: EventEnvelope<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionWindow {
    pub oldest_sequence: u64,
    pub newest_sequence: u64,
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

    pub fn retention_window(&self) -> Option<RetentionWindow> {
        Some(RetentionWindow {
            oldest_sequence: self.events.front()?.sequence,
            newest_sequence: self.events.back()?.sequence,
        })
    }

    pub fn has_retention_gap(&self, requested_sequence: u64) -> bool {
        self.retention_window()
            .is_some_and(|window| requested_sequence < window.oldest_sequence)
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

    #[test]
    fn reports_retention_gap_after_capacity_eviction() {
        let mut bus = EventBus::new(2);
        let first = bus.publish(sample_event("service"));
        let second = bus.publish(sample_event("worker"));
        let third = bus.publish(sample_event("mcp"));

        assert_eq!(
            bus.retention_window(),
            Some(RetentionWindow {
                oldest_sequence: second,
                newest_sequence: third,
            })
        );
        assert!(bus.has_retention_gap(first));
        assert!(!bus.has_retention_gap(second));

        let retained_sequences = bus
            .read_from(first)
            .into_iter()
            .map(|entry| entry.sequence)
            .collect::<Vec<_>>();
        assert_eq!(retained_sequences, vec![second, third]);
    }
}
