#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_outbox::OutboxRecord;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CdcRecord {
    pub offset: u64,
    pub event_id: String,
    pub event_type: String,
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CdcStream {
    pub records: Vec<CdcRecord>,
}

impl CdcStream {
    #[must_use]
    pub fn from_outbox(records: &[OutboxRecord]) -> Self {
        Self {
            records: records
                .iter()
                .map(|record| CdcRecord {
                    offset: record.sequence,
                    event_id: record.envelope.event_id.clone(),
                    event_type: record.envelope.event_type.clone(),
                    payload: record.envelope.payload.clone(),
                })
                .collect(),
        }
    }

    #[must_use]
    pub fn after(&self, offset: u64) -> Vec<CdcRecord> {
        self.records
            .iter()
            .filter(|record| record.offset > offset)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_event::sample_event;
    use tdw_outbox::InMemoryOutbox;

    #[test]
    fn converts_outbox_records_to_ordered_cdc_offsets() {
        let mut outbox = InMemoryOutbox::default();
        outbox.append(sample_event("service"));
        outbox.append(sample_event("worker"));

        let cdc = CdcStream::from_outbox(&outbox.pending_after(0));
        assert_eq!(cdc.records.len(), 2);
        assert_eq!(cdc.after(1)[0].offset, 2);
    }
}
