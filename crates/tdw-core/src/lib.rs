#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid query: {0}")]
    InvalidQuery(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("registry error: {0}")]
    Registry(String),
}

pub trait QueryParams:
    Clone + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static
{
}

impl<T> QueryParams for T where
    T: Clone + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static
{
}

pub trait DataModel:
    Clone + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static
{
}

impl<T> DataModel for T where
    T: Clone + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static
{
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
pub struct OBBject<T: DataModel> {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub rows: Vec<T>,
    pub metadata: BTreeMap<String, Value>,
}

impl<T: DataModel> OBBject<T> {
    pub fn new(rows: Vec<T>, provider: &'static str, endpoint: &'static str) -> Self {
        Self {
            provider,
            endpoint,
            rows,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub polygon_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub google_api_key: Option<String>,
}

#[async_trait]
pub trait Fetcher: Send + Sync + 'static {
    type Query: QueryParams;
    type Data: DataModel;

    const PROVIDER: &'static str;
    const ENDPOINT: &'static str;

    fn transform_query(params: Value) -> Result<Self::Query>;
    async fn extract_data(&self, query: &Self::Query, creds: &Credentials) -> Result<Bytes>;
    fn transform_data(&self, query: &Self::Query, raw: Bytes) -> Result<Vec<Self::Data>>;

    async fn fetch(&self, params: Value, creds: &Credentials) -> Result<OBBject<Self::Data>> {
        let query = Self::transform_query(params)?;
        let raw = self.extract_data(&query, creds).await?;
        let rows = self.transform_data(&query, raw)?;
        Ok(OBBject::new(rows, Self::PROVIDER, Self::ENDPOINT))
    }
}

pub type DataStream<T> = Pin<Box<dyn Stream<Item = Result<T>> + Send>>;

#[async_trait]
pub trait Streamer: Send + Sync + 'static {
    type Query: QueryParams;
    type Data: DataModel;

    const PROVIDER: &'static str;
    const ENDPOINT: &'static str;

    async fn subscribe(
        &self,
        query: Self::Query,
        creds: &Credentials,
    ) -> Result<DataStream<Self::Data>>;
    async fn snapshot(&self, query: &Self::Query, creds: &Credentials) -> Result<Vec<Self::Data>>;
    async fn checkpoint(&self, _seq: u64) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteReceipt {
    pub sink: &'static str,
    pub rows_written: usize,
}

#[async_trait]
pub trait WriteSink<T: DataModel>: Send + Sync {
    fn name(&self) -> &'static str;
    async fn write_batch(&self, batch: &OBBject<T>) -> Result<WriteReceipt>;
    async fn health_check(&self) -> Result<HealthStatus>;
}

#[async_trait]
pub trait OlapEngine: Send + Sync {
    async fn execute(&self, ddl: &str) -> Result<()>;
    async fn query_json(&self, sql: &str, params: Value) -> Result<Value>;
}

#[async_trait]
pub trait RelationalEngine: Send + Sync {
    async fn execute(&self, sql: &str, params: Value) -> Result<u64>;
    async fn fetch_json(&self, sql: &str, params: Value) -> Result<Vec<Value>>;
}

#[async_trait]
pub trait VectorEngine: Send + Sync {
    async fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<()>;
    async fn search_knn(&self, collection: &str, query: VectorQuery) -> Result<Vec<ScoredPoint>>;
}

#[async_trait]
pub trait LexicalEngine: Send + Sync {
    async fn index(&self, index: &str, docs: Vec<LexicalDoc>) -> Result<()>;
    async fn search_text(&self, index: &str, query: TextQuery) -> Result<Vec<ScoredDoc>>;
}

#[async_trait]
pub trait BlobEngine: Send + Sync {
    async fn put_object(&self, key: &str, body: Bytes, content_type: &str) -> Result<()>;
    async fn get_object(&self, key: &str) -> Result<Bytes>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorPoint {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorQuery {
    pub vector: Vec<f32>,
    pub top_k: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoredPoint {
    pub id: String,
    pub score: f32,
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LexicalDoc {
    pub id: String,
    pub body: String,
    pub fields: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextQuery {
    pub text: String,
    pub top_k: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoredDoc {
    pub id: String,
    pub score: f32,
    pub fields: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProgressOrResult<T: DataModel> {
    Progress {
        stage: &'static str,
        fraction: f32,
        message: Option<String>,
    },
    Partial(T),
    Done(OBBject<T>),
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    Fetcher,
    Streamer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub provider: &'static str,
    pub endpoint: &'static str,
    pub kind: ProviderKind,
}

#[derive(Default, Clone, Debug)]
pub struct ProviderRegistry {
    entries: Vec<RegistryEntry>,
}

impl ProviderRegistry {
    pub fn register(&mut self, entry: RegistryEntry) -> Result<()> {
        if self.entries.iter().any(|existing| {
            existing.provider == entry.provider
                && existing.endpoint == entry.endpoint
                && existing.kind == entry.kind
        }) {
            return Err(Error::Registry(format!(
                "duplicate provider registration: {}/{}",
                entry.provider, entry.endpoint
            )));
        }
        self.entries.push(entry);
        Ok(())
    }

    pub fn entries(&self) -> &[RegistryEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    struct Row {
        symbol: String,
    }

    #[test]
    fn envelope_preserves_provider_and_endpoint() {
        let rows = vec![Row {
            symbol: "AAPL".to_string(),
        }];
        let object = OBBject::new(rows, "fileset", "equity_historical");

        assert_eq!(object.provider, "fileset");
        assert_eq!(object.endpoint, "equity_historical");
        assert_eq!(object.rows[0].symbol, "AAPL");
    }

    #[test]
    fn registry_rejects_duplicate_entries() {
        let mut registry = ProviderRegistry::default();
        let entry = RegistryEntry {
            provider: "fileset",
            endpoint: "equity_historical",
            kind: ProviderKind::Fetcher,
        };

        assert!(registry.register(entry.clone()).is_ok());
        let duplicate = registry.register(entry);

        assert!(duplicate.is_err());
    }
}
