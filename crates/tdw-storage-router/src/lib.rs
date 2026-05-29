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
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_sink(&mut self, sink: Arc<dyn WriteSink<T>>) {
        self.sinks.push(sink);
    }

    #[must_use]
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
        if self.sinks.is_empty() {
            return Err(Error::Storage(
                "storage-router has no registered sinks".to_string(),
            ));
        }
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
        if self.sinks.is_empty() {
            return Ok(HealthStatus::Degraded(
                "storage-router has no registered sinks".to_string(),
            ));
        }
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
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            writes: Mutex::new(0),
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
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
        drop(writes);
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

    #[tokio::test]
    async fn empty_router_rejects_writes_and_reports_degraded() {
        let router = StorageRouter::<Row>::new();
        let batch = OBBject::new(
            vec![Row {
                symbol: "AAPL".to_string(),
            }],
            "test",
            "rows",
        );

        assert!(router.write_batch(&batch).await.is_err());
        let status = router
            .health_check()
            .await
            .unwrap_or_else(|error| panic!("health check should return status: {error}"));

        assert_eq!(
            status,
            HealthStatus::Degraded("storage-router has no registered sinks".to_string())
        );
    }

    #[tokio::test]
    async fn router_fans_out_writes_and_sums_receipts() {
        let first = Arc::new(RecordingSink::new("first"));
        let second = Arc::new(RecordingSink::new("second"));
        let mut router = StorageRouter::<Row>::new();
        router.add_sink(first.clone());
        router.add_sink(second.clone());
        let batch = OBBject::new(
            vec![Row {
                symbol: "AAPL".to_string(),
            }],
            "test",
            "rows",
        );

        let receipt = router
            .write_batch(&batch)
            .await
            .unwrap_or_else(|error| panic!("router should write: {error}"));

        assert_eq!(receipt.rows_written, 2);
        assert_eq!(
            first
                .writes()
                .unwrap_or_else(|error| panic!("first sink should count writes: {error}")),
            1
        );
        assert_eq!(
            second
                .writes()
                .unwrap_or_else(|error| panic!("second sink should count writes: {error}")),
            1
        );
    }
}
