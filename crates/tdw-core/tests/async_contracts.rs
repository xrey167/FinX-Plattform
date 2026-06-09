#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Integration coverage for the async trait surface of tdw-core: the default
// `Fetcher::fetch` pipeline, `Streamer` snapshot/subscribe/default-checkpoint,
// and `WriteSink` write/health, plus the serde and equality behaviour of the
// public envelope and payload types. These exercise the public API only, so the
// lines they cover live in `src/lib.rs` while the test bodies themselves are
// excluded from the scoped coverage denominator.

use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tdw_core::{
    Credentials, DataStream, Error, Fetcher, HealthStatus, LexicalDoc, OBBject, ProgressOrResult,
    ProviderKind, RegistryEntry, ScoredDoc, ScoredPoint, Streamer, TextQuery, VectorPoint,
    VectorQuery, WriteReceipt, WriteSink,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Row {
    symbol: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
struct Query {
    symbol: String,
}

struct MockFetcher;

#[async_trait]
impl Fetcher<Query, Row> for MockFetcher {
    const PROVIDER: &'static str = "mock";
    const ENDPOINT: &'static str = "equity_historical";

    fn transform_query(params: Value) -> tdw_core::Result<Query> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("missing symbol".to_string()))?;
        Ok(Query {
            symbol: symbol.to_string(),
        })
    }

    async fn extract_data(&self, query: &Query, _creds: &Credentials) -> tdw_core::Result<Bytes> {
        Ok(Bytes::from(query.symbol.clone()))
    }

    fn transform_data(&self, _query: &Query, raw: Bytes) -> tdw_core::Result<Vec<Row>> {
        let symbol =
            String::from_utf8(raw.to_vec()).map_err(|error| Error::Provider(error.to_string()))?;
        Ok(vec![Row { symbol }])
    }
}

struct MockStreamer;

#[async_trait]
impl Streamer<Query, Row> for MockStreamer {
    const PROVIDER: &'static str = "mock-ws";
    const ENDPOINT: &'static str = "equity_ticks";

    async fn subscribe(
        &self,
        _query: Query,
        _creds: &Credentials,
    ) -> tdw_core::Result<DataStream<Row>> {
        Ok(Box::pin(EmptyStream))
    }

    async fn snapshot(&self, query: &Query, _creds: &Credentials) -> tdw_core::Result<Vec<Row>> {
        Ok(vec![Row {
            symbol: query.symbol.clone(),
        }])
    }
}

struct CountingSink;

#[async_trait]
impl WriteSink<Row> for CountingSink {
    fn name(&self) -> &'static str {
        "counting"
    }

    async fn write_batch(&self, batch: &OBBject<Row>) -> tdw_core::Result<WriteReceipt> {
        if batch.rows.is_empty() {
            return Err(Error::Storage("empty batch".to_string()));
        }
        Ok(WriteReceipt {
            sink: "counting",
            rows_written: batch.rows.len(),
        })
    }

    async fn health_check(&self) -> tdw_core::Result<HealthStatus> {
        Ok(HealthStatus::Degraded("warming up".to_string()))
    }
}

struct EmptyStream;

impl Stream for EmptyStream {
    type Item = tdw_core::Result<Row>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(None)
    }
}

#[test]
fn envelope_round_trips_json() {
    let object = OBBject::new(
        vec![Row {
            symbol: "MSFT".to_string(),
        }],
        "fileset",
        "equity_historical",
    )
    .with_metadata("source", json!("golden"));

    let json = serde_json::to_string(&object).unwrap();
    let round_trip: OBBject<Row> = serde_json::from_str(&json).unwrap();
    assert_eq!(round_trip, object);
}

#[tokio::test]
async fn fetcher_default_fetch_pipes_query_extract_and_transform() {
    let fetcher = MockFetcher;
    let creds = Credentials::default();

    let object = fetcher
        .fetch(json!({ "symbol": "TSLA" }), &creds)
        .await
        .unwrap();

    assert_eq!(object.provider, "mock");
    assert_eq!(object.endpoint, "equity_historical");
    assert_eq!(
        object.rows,
        vec![Row {
            symbol: "TSLA".to_string()
        }]
    );
    assert!(object.metadata.is_empty());
}

#[tokio::test]
async fn fetcher_default_fetch_propagates_transform_query_error() {
    let fetcher = MockFetcher;
    let creds = Credentials::default();

    let error = fetcher
        .fetch(json!({ "not_symbol": "x" }), &creds)
        .await
        .expect_err("missing symbol must error");

    assert!(matches!(error, Error::InvalidQuery(message) if message == "missing symbol"));
}

#[tokio::test]
async fn fetcher_transform_data_reports_invalid_utf8_as_provider_error() {
    let fetcher = MockFetcher;
    let query = Query {
        symbol: "ignored".to_string(),
    };
    let raw = Bytes::from_static(&[0xff, 0xfe, 0xfd]);

    let error = fetcher
        .transform_data(&query, raw)
        .expect_err("invalid utf-8 must error");
    assert!(
        matches!(error, Error::Provider(message) if message.contains("utf-8") || message.contains("UTF-8"))
    );
}

#[tokio::test]
async fn streamer_snapshot_and_subscribe_and_default_checkpoint() {
    let streamer = MockStreamer;
    let creds = Credentials::default();
    let query = Query {
        symbol: "NVDA".to_string(),
    };

    let snapshot = streamer.snapshot(&query, &creds).await.unwrap();
    assert_eq!(
        snapshot,
        vec![Row {
            symbol: "NVDA".to_string()
        }]
    );

    let mut stream = streamer.subscribe(query, &creds).await.unwrap();
    assert!(stream.next().await.is_none());

    // Default checkpoint implementation is a no-op that returns Ok.
    streamer.checkpoint(42).await.unwrap();
}

#[tokio::test]
async fn write_sink_reports_name_receipt_and_health() {
    let sink = CountingSink;
    assert_eq!(sink.name(), "counting");

    let batch = OBBject::new(
        vec![
            Row {
                symbol: "AAPL".to_string(),
            },
            Row {
                symbol: "MSFT".to_string(),
            },
        ],
        "mock",
        "equity_historical",
    );
    let receipt = sink.write_batch(&batch).await.unwrap();
    assert_eq!(receipt.sink, "counting");
    assert_eq!(receipt.rows_written, 2);
    assert_eq!(receipt.clone(), receipt);

    let empty: OBBject<Row> = OBBject::new(Vec::new(), "mock", "equity_historical");
    let error = sink
        .write_batch(&empty)
        .await
        .expect_err("empty batch must error");
    assert!(matches!(error, Error::Storage(message) if message == "empty batch"));

    let health = sink.health_check().await.unwrap();
    assert_eq!(health, HealthStatus::Degraded("warming up".to_string()));
}

#[test]
fn write_receipt_round_trips_json() {
    let receipt = WriteReceipt {
        sink: "clickhouse",
        rows_written: 7,
    };
    let value = serde_json::to_value(&receipt).unwrap();
    assert_eq!(
        value.get("sink").and_then(Value::as_str),
        Some("clickhouse")
    );
    assert_eq!(value.get("rows_written").and_then(Value::as_u64), Some(7));
    assert_eq!(receipt.clone(), receipt);
}

#[test]
fn provider_kind_and_registry_entry_serialize_and_clone() {
    let entry = RegistryEntry::streamer("mock-ws", "equity_ticks");
    let value = serde_json::to_value(&entry).unwrap();

    assert_eq!(
        value.get("provider").and_then(Value::as_str),
        Some("mock-ws")
    );
    assert_eq!(
        value.get("endpoint").and_then(Value::as_str),
        Some("equity_ticks")
    );
    assert_eq!(entry.kind, ProviderKind::Streamer);
    assert_eq!(entry.clone(), entry);

    let kind_value = serde_json::to_value(ProviderKind::Fetcher).unwrap();
    assert_eq!(kind_value.as_str(), Some("Fetcher"));
    let parsed_kind: ProviderKind = serde_json::from_value(kind_value).unwrap();
    assert_eq!(parsed_kind, ProviderKind::Fetcher);
}

#[test]
fn health_status_equality_distinguishes_degraded_reason() {
    assert_eq!(HealthStatus::Healthy, HealthStatus::Healthy);
    assert_ne!(
        HealthStatus::Degraded("lag".to_string()),
        HealthStatus::Degraded("offline".to_string())
    );
    assert_ne!(
        HealthStatus::Healthy,
        HealthStatus::Degraded("x".to_string())
    );
}

#[test]
fn vector_and_lexical_payloads_round_trip_json() {
    let point = VectorPoint {
        id: "v1".to_string(),
        vector: vec![0.1, 0.2],
        payload: json!({ "label": "a" }),
    };
    let query = VectorQuery {
        vector: vec![0.1, 0.2],
        top_k: 3,
    };
    let scored = ScoredPoint {
        id: "v1".to_string(),
        score: 0.9,
        payload: json!({ "label": "a" }),
    };
    let doc = LexicalDoc {
        id: "d1".to_string(),
        body: "hello".to_string(),
        fields: json!({ "lang": "en" }),
    };
    let text_query = TextQuery {
        text: "hello".to_string(),
        top_k: 5,
    };
    let scored_doc = ScoredDoc {
        id: "d1".to_string(),
        score: 0.5,
        fields: json!({ "lang": "en" }),
    };

    for value in [
        serde_json::to_value(&point).unwrap(),
        serde_json::to_value(&query).unwrap(),
        serde_json::to_value(&scored).unwrap(),
        serde_json::to_value(&doc).unwrap(),
        serde_json::to_value(&text_query).unwrap(),
        serde_json::to_value(&scored_doc).unwrap(),
    ] {
        assert!(value.is_object());
    }

    let point_back: VectorPoint =
        serde_json::from_value(serde_json::to_value(&point).unwrap()).unwrap();
    assert_eq!(point_back, point);
    let scored_back: ScoredPoint =
        serde_json::from_value(serde_json::to_value(&scored).unwrap()).unwrap();
    assert_eq!(scored_back, scored);
}

#[test]
fn progress_or_result_variants_are_distinct() {
    let progress: ProgressOrResult<Row> = ProgressOrResult::Progress {
        stage: "extract",
        fraction: 0.5,
        message: Some("halfway".to_string()),
    };
    let partial: ProgressOrResult<Row> = ProgressOrResult::Partial(Row {
        symbol: "AAPL".to_string(),
    });
    let done: ProgressOrResult<Row> = ProgressOrResult::Done(OBBject::new(
        vec![Row {
            symbol: "AAPL".to_string(),
        }],
        "mock",
        "equity_historical",
    ));
    let error: ProgressOrResult<Row> = ProgressOrResult::Error("boom".to_string());

    assert_ne!(progress, partial);
    assert_ne!(partial, done);
    assert_ne!(done, error);
    assert_eq!(progress.clone(), progress);
    assert!(matches!(error, ProgressOrResult::Error(message) if message == "boom"));
}

#[test]
fn credentials_default_is_all_none_and_clone_matches() {
    let creds = Credentials::default();
    assert!(creds.polygon_api_key.is_none());
    assert!(creds.openai_api_key.is_none());
    assert!(creds.google_api_key.is_none());
    assert!(creds.anthropic_api_key.is_none());

    let populated = Credentials {
        polygon_api_key: Some("pk".to_string()),
        ..Credentials::default()
    };
    assert_eq!(populated.clone(), populated);
    assert_ne!(populated, creds);
}

#[test]
fn registry_register_fetcher_and_streamer_helpers_record_constants() {
    use tdw_core::ProviderRegistry;

    let mut registry = ProviderRegistry::default();
    registry
        .register_fetcher::<MockFetcher, Query, Row>()
        .unwrap();
    registry
        .register_streamer::<MockStreamer, Query, Row>()
        .unwrap();

    assert!(registry.contains("mock", "equity_historical", ProviderKind::Fetcher));
    assert!(registry.contains("mock-ws", "equity_ticks", ProviderKind::Streamer));
}

#[test]
fn from_inventory_without_feature_yields_empty_registry() {
    use tdw_core::ProviderRegistry;

    let registry = ProviderRegistry::from_inventory().unwrap();
    assert!(registry.entries().is_empty());
    assert!(!registry.contains("any", "thing", ProviderKind::Fetcher));
}
