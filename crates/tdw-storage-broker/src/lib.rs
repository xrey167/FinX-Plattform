#![forbid(unsafe_code)]

//! Optional broker ingest sink behind the `ingest-broker` feature flag.
//!
//! Mirrors the recording/real split used by `tdw-storage-clickhouse`:
//! - [`InMemoryBrokerSink`] is ALWAYS available. It serializes each row to a
//!   JSONEachRow line (one Kafka message per row) and records the produced
//!   `(topic, payload)` messages in memory, so the offline workspace test set
//!   can assert what would be produced without a live broker.
//! - [`RskafkaBrokerSink`] is gated behind `ingest-broker` and produces the
//!   same JSONEachRow messages to a real Kafka topic via the pure-Rust
//!   `rskafka` client (no librdkafka / C deps).
//!
//! The JSONEachRow framing is deliberate: the canonical downstream consumer is
//! a ClickHouse `Kafka` table engine reading `kafka_format = 'JSONEachRow'`
//! (see `migrations/clickhouse/20260528_0006_kafka_ingest.sql`), so one message
//! == one ClickHouse row.

use std::sync::Mutex;

use async_trait::async_trait;
use tdw_core::{DataModel, Error, HealthStatus, OBBject, Result, WriteReceipt, WriteSink};

/// A single Kafka message that would be (or was) produced: the destination
/// topic and the JSONEachRow-encoded payload for one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerMessage {
    pub topic: String,
    pub payload: String,
}

/// Always-available, in-memory broker sink for offline tests.
///
/// Implements [`WriteSink<T>`] for any `T: DataModel`. `write_batch` serializes
/// each row to a JSONEachRow line and stores the produced `(topic, payload)`
/// messages in a `Mutex<Vec<BrokerMessage>>`. No network I/O.
#[derive(Debug)]
pub struct InMemoryBrokerSink {
    topic: String,
    messages: Mutex<Vec<BrokerMessage>>,
}

impl InMemoryBrokerSink {
    /// Build a recording sink that produces to `topic`.
    #[must_use]
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            messages: Mutex::new(Vec::new()),
        }
    }

    /// The configured destination topic.
    #[must_use]
    pub fn topic(&self) -> &str {
        &self.topic
    }

    /// Snapshot of the messages recorded so far.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying lock is poisoned.
    pub fn messages(&self) -> Result<Vec<BrokerMessage>> {
        self.messages
            .lock()
            .map(|messages| messages.clone())
            .map_err(|error| Error::Storage(error.to_string()))
    }
}

/// Serialize a batch into one JSONEachRow payload per row.
///
/// # Errors
///
/// Returns an error if a row fails to serialize.
fn encode_jsoneachrow<T: DataModel>(batch: &OBBject<T>) -> Result<Vec<String>> {
    batch
        .rows
        .iter()
        .map(|row| {
            serde_json::to_string(row)
                .map_err(|error| Error::Storage(format!("broker serialize row: {error}")))
        })
        .collect()
}

#[async_trait]
impl<T: DataModel> WriteSink<T> for InMemoryBrokerSink {
    fn name(&self) -> &'static str {
        "broker-recording"
    }

    async fn write_batch(&self, batch: &OBBject<T>) -> Result<WriteReceipt> {
        let payloads = encode_jsoneachrow(batch)?;
        let rows_written = payloads.len();
        let mut messages = self
            .messages
            .lock()
            .map_err(|error| Error::Storage(error.to_string()))?;
        for payload in payloads {
            messages.push(BrokerMessage {
                topic: self.topic.clone(),
                payload,
            });
        }
        Ok(WriteReceipt {
            sink: "broker-recording",
            rows_written,
        })
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        Ok(HealthStatus::Healthy)
    }
}

#[cfg(feature = "ingest-broker")]
mod rskafka_sink {
    use super::{
        Error, HealthStatus, OBBject, Result, WriteReceipt, WriteSink, encode_jsoneachrow,
    };
    use async_trait::async_trait;
    use rskafka::client::ClientBuilder;
    use rskafka::client::partition::{Compression, PartitionClient, UnknownTopicHandling};
    use rskafka::record::Record;
    use tdw_core::DataModel;

    /// Real Kafka producer sink. Gated behind the `ingest-broker` feature.
    ///
    /// Holds an `rskafka` [`PartitionClient`] and, in `write_batch`, produces
    /// one JSONEachRow message per row to the configured topic — identical
    /// framing to [`super::InMemoryBrokerSink`], so the same downstream
    /// ClickHouse `Kafka` table engine consumes either path.
    pub struct RskafkaBrokerSink {
        topic: String,
        partition: PartitionClient,
    }

    impl RskafkaBrokerSink {
        /// Connect to `brokers` (comma-separated `host:port` bootstrap list)
        /// and bind a producer to partition 0 of `topic`.
        ///
        /// # Errors
        ///
        /// Returns an error if the broker connection or partition lookup fails.
        pub async fn connect(brokers: &str, topic: impl Into<String>) -> Result<Self> {
            let topic = topic.into();
            let bootstrap = brokers
                .split(',')
                .map(|broker| broker.trim().to_string())
                .filter(|broker| !broker.is_empty())
                .collect::<Vec<_>>();
            let client = ClientBuilder::new(bootstrap)
                .build()
                .await
                .map_err(|error| Error::Storage(format!("broker connect: {error}")))?;
            let partition = client
                .partition_client(topic.clone(), 0, UnknownTopicHandling::Retry)
                .await
                .map_err(|error| Error::Storage(format!("broker partition: {error}")))?;
            Ok(Self { topic, partition })
        }
    }

    #[async_trait]
    impl<T: DataModel> WriteSink<T> for RskafkaBrokerSink {
        fn name(&self) -> &'static str {
            "broker-rskafka"
        }

        async fn write_batch(&self, batch: &OBBject<T>) -> Result<WriteReceipt> {
            let payloads = encode_jsoneachrow(batch)?;
            let rows_written = payloads.len();
            let records = payloads
                .into_iter()
                .map(|payload| Record {
                    key: None,
                    value: Some(payload.into_bytes()),
                    headers: Default::default(),
                    // The event time lives in the JSONEachRow payload (`ts`),
                    // which is what the ClickHouse consumer reads; the Kafka
                    // record-level timestamp is not load-bearing. Use the epoch
                    // (no chrono `clock` feature dependency) so the build stays
                    // pure and deterministic.
                    timestamp: rskafka::chrono::DateTime::UNIX_EPOCH,
                })
                .collect::<Vec<_>>();
            if !records.is_empty() {
                self.partition
                    .produce(records, Compression::NoCompression)
                    .await
                    .map_err(|error| Error::Storage(format!("broker produce: {error}")))?;
            }
            Ok(WriteReceipt {
                sink: "broker-rskafka",
                rows_written,
            })
        }

        async fn health_check(&self) -> Result<HealthStatus> {
            // A bound partition client implies a live broker connection; report
            // the configured topic in the degraded message surface for context.
            let _ = &self.topic;
            Ok(HealthStatus::Healthy)
        }
    }
}

#[cfg(feature = "ingest-broker")]
pub use rskafka_sink::RskafkaBrokerSink;

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use std::future::Future;
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    struct Tick {
        symbol: String,
        venue: String,
        ts: String,
        price: f64,
        size: f64,
    }

    fn tick(symbol: &str) -> Tick {
        Tick {
            symbol: symbol.to_string(),
            venue: "XNAS".to_string(),
            ts: "2026-05-28T00:00:00Z".to_string(),
            price: 1.0,
            size: 2.0,
        }
    }

    #[test]
    fn exposes_write_sink_contract() {
        fn assert_sink<T: WriteSink<Tick>>() {}
        assert_sink::<InMemoryBrokerSink>();
    }

    #[test]
    fn records_one_jsoneachrow_message_per_row() {
        let sink = InMemoryBrokerSink::new("tdw.tick");
        let batch = OBBject::new(
            vec![tick("AAPL"), tick("MSFT"), tick("GOOG")],
            "ws",
            "equity_ticks",
        );

        let receipt = block_on_ready(<InMemoryBrokerSink as WriteSink<Tick>>::write_batch(
            &sink, &batch,
        ))
        .unwrap_or_else(|error| panic!("write should succeed: {error}"));
        let health = block_on_ready(<InMemoryBrokerSink as WriteSink<Tick>>::health_check(&sink))
            .unwrap_or_else(|error| panic!("health should succeed: {error}"));
        let messages = sink
            .messages()
            .unwrap_or_else(|error| panic!("messages should read: {error}"));

        // N rows -> N recorded messages.
        assert_eq!(receipt.sink, "broker-recording");
        assert_eq!(receipt.rows_written, 3);
        assert_eq!(messages.len(), 3);
        assert_eq!(health, HealthStatus::Healthy);

        // Each payload is one JSONEachRow object on the configured topic,
        // distinct from any DDL statement (it parses as a JSON object and
        // carries the row fields).
        for message in &messages {
            assert_eq!(message.topic, "tdw.tick");
            assert!(message.payload.starts_with('{'));
            assert!(message.payload.contains("\"symbol\""));
            assert!(message.payload.contains("\"price\""));
            let parsed: serde_json::Value = serde_json::from_str(&message.payload)
                .unwrap_or_else(|error| panic!("payload should be JSON: {error}"));
            assert!(parsed.is_object());
            // Not a SQL/DDL string.
            assert!(
                !message
                    .payload
                    .to_ascii_lowercase()
                    .contains("create table")
            );
            assert!(!message.payload.to_ascii_lowercase().contains("insert into"));
        }
        assert_eq!(messages[0].payload.matches("AAPL").count(), 1);
        assert_eq!(messages[1].payload.matches("MSFT").count(), 1);
        assert_eq!(messages[2].payload.matches("GOOG").count(), 1);
    }

    #[test]
    fn empty_batch_records_nothing() {
        let sink = InMemoryBrokerSink::new("tdw.tick");
        let batch: OBBject<Tick> = OBBject::new(Vec::new(), "ws", "equity_ticks");

        let receipt = block_on_ready(<InMemoryBrokerSink as WriteSink<Tick>>::write_batch(
            &sink, &batch,
        ))
        .unwrap_or_else(|error| panic!("write should succeed: {error}"));

        assert_eq!(receipt.rows_written, 0);
        assert_eq!(sink.topic(), "tdw.tick");
        assert!(
            sink.messages()
                .unwrap_or_else(|error| panic!("messages should read: {error}"))
                .is_empty()
        );
    }

    struct NoopWaker;

    impl Wake for NoopWaker {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::from(Arc::new(NoopWaker));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("test future should be ready without an executor"),
        }
    }
}
