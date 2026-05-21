#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tdw_core::{DataModel, Error, HealthStatus, OBBject, Result, WriteReceipt, WriteSink};

#[derive(Clone)]
pub struct StorageRouter<T: DataModel> {
    sinks: Vec<Arc<dyn WriteSink<T>>>,
}

impl<T: DataModel> Default for StorageRouter<T> {
    fn default() -> Self {
        Self { sinks: Vec::new() }
    }
}

impl<T: DataModel> StorageRouter<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_sink(&mut self, sink: Arc<dyn WriteSink<T>>) {
        self.sinks.push(sink);
    }

    pub fn sink_count(&self) -> usize {
        self.sinks.len()
    }
}

#[async_trait]
impl<T: DataModel> WriteSink<T> for StorageRouter<T> {
    fn name(&self) -> &'static str {
        "storage-router"
    }

    async fn write_batch(&self, batch: &OBBject<T>) -> Result<WriteReceipt> {
        let mut rows_written = 0;
        for sink in &self.sinks {
            let receipt = sink.write_batch(batch).await?;
            rows_written += receipt.rows_written;
        }
        Ok(WriteReceipt {
            sink: self.name(),
            rows_written,
        })
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        for sink in &self.sinks {
            match sink.health_check().await? {
                HealthStatus::Healthy => {}
                status => return Ok(status),
            }
        }
        Ok(HealthStatus::Healthy)
    }
}

#[derive(Debug)]
pub struct RecordingSink {
    name: &'static str,
    writes: Mutex<usize>,
}

impl RecordingSink {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            writes: Mutex::new(0),
        }
    }

    pub fn writes(&self) -> Result<usize> {
        self.writes
            .lock()
            .map(|writes| *writes)
            .map_err(|error| Error::Storage(error.to_string()))
    }
}

#[async_trait]
impl<T: DataModel> WriteSink<T> for RecordingSink {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn write_batch(&self, batch: &OBBject<T>) -> Result<WriteReceipt> {
        let mut writes = self
            .writes
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        *writes += batch.rows.len();
        Ok(WriteReceipt {
            sink: self.name,
            rows_written: batch.rows.len(),
        })
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        Ok(HealthStatus::Healthy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    struct Row {
        symbol: String,
    }

    #[test]
    fn router_accepts_specialist_sinks() {
        let mut router = StorageRouter::<Row>::new();
        router.add_sink(Arc::new(RecordingSink::new("a")));
        router.add_sink(Arc::new(RecordingSink::new("b")));

        assert_eq!(router.sink_count(), 2);
    }
}
