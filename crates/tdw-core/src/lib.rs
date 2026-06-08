#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::{Component, Path};
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

/// Validate a SQL identifier (e.g. a table name) that must be interpolated
/// into a statement because SQL has no bind parameter for identifiers.
///
/// This is the shared form of the `tdw-storage-clickhouse` reference validator,
/// intended for every store that derives a table name from operator/config
/// input — notably the Postgres `with_table` constructors. Accepts ASCII
/// alphanumerics, `_`, and `.` (for schema-qualified names), requires an
/// alphabetic or `_` lead, bounds the length, and rejects empty input or
/// malformed dot placement (leading, trailing, or doubled dots). An identifier
/// containing `;`, `"`, `--`, whitespace, or a leading digit is rejected, so it
/// cannot inject SQL when `format!`-interpolated.
///
/// # Errors
///
/// Returns [`Error::Storage`] if `name` is not a valid SQL identifier.
pub fn safe_sql_identifier(name: &str) -> Result<()> {
    const MAX_LEN: usize = 64;
    let valid = !name.is_empty()
        && name.len() <= MAX_LEN
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..");
    if valid {
        Ok(())
    } else {
        Err(Error::Storage(format!(
            "invalid SQL identifier {name:?}: expected [A-Za-z_][A-Za-z0-9_.]* \
             (1..={MAX_LEN} chars, no leading/trailing/doubled dots)"
        )))
    }
}

/// Validate a single safe path *segment* (a filename with no directory parts)
/// before it is interpolated into a path, e.g. `dir.join(format!("{name}.json5"))`.
///
/// Requires a non-empty, control-free string that contains no `/` or `\` and
/// resolves to exactly one [`Component::Normal`] equal to the input — so `.`,
/// `..`, absolute roots, and Windows drive prefixes are all rejected. Use for
/// values that must stay a single filename (e.g. an agent-store memory name)
/// so a crafted name cannot write or delete outside the store directory.
///
/// # Errors
///
/// Returns [`Error::Storage`] if `name` is not a safe single path segment.
pub fn safe_path_segment(name: &str) -> Result<()> {
    let mut components = Path::new(name).components();
    let single_normal = matches!(
        components.next(),
        Some(Component::Normal(segment)) if segment == std::ffi::OsStr::new(name)
    ) && components.next().is_none();
    if !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(char::is_control)
        && single_normal
    {
        Ok(())
    } else {
        Err(Error::Storage(format!(
            "invalid path segment {name:?}: must be a single filename with no \
             separators, traversal, root, or control characters"
        )))
    }
}

/// Validate a safe *relative* path that must stay within a trusted base
/// directory before it is opened or joined onto that base.
///
/// Mirrors the `tdw-storage-fs` reference validator: rejects empty/control or
/// backslash-bearing inputs and any path whose [`Component`]s include
/// [`Component::CurDir`] (`.`), [`Component::ParentDir`] (`..`),
/// [`Component::RootDir`] (a leading `/`), or [`Component::Prefix`] (a Windows
/// drive). Multi-segment relative paths like `a/b` are allowed. This is
/// stronger than a `contains("..")` check because it also blocks absolute
/// paths (e.g. `/etc/shadow`) and drive prefixes.
///
/// # Errors
///
/// Returns [`Error::Storage`] if `path` is not a safe relative path.
pub fn safe_relative_path(path: &str) -> Result<()> {
    let traversal = Path::new(path).components().any(|component| {
        matches!(
            component,
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if !path.trim().is_empty()
        && !path.contains('\\')
        && !path.chars().any(char::is_control)
        && !traversal
    {
        Ok(())
    } else {
        Err(Error::Storage(format!(
            "invalid relative path {path:?}: must be a non-empty, control-free \
             relative path with no '..', absolute root, or drive prefix"
        )))
    }
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
pub struct OBBject<T: DataModel> {
    pub provider: String,
    pub endpoint: String,
    pub rows: Vec<T>,
    pub metadata: BTreeMap<String, Value>,
}

impl<T: DataModel> OBBject<T> {
    #[must_use]
    pub fn new(rows: Vec<T>, provider: &'static str, endpoint: &'static str) -> Self {
        Self {
            provider: provider.to_string(),
            endpoint: endpoint.to_string(),
            rows,
            metadata: BTreeMap::new(),
        }
    }

    #[must_use]
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
    pub anthropic_api_key: Option<String>,
}

#[async_trait]
pub trait Fetcher<Q, D>: Send + Sync + 'static
where
    Q: QueryParams,
    D: DataModel,
{
    const PROVIDER: &'static str;
    const ENDPOINT: &'static str;

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn transform_query(params: Value) -> Result<Q>;
    async fn extract_data(&self, query: &Q, creds: &Credentials) -> Result<Bytes>;
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    fn transform_data(&self, query: &Q, raw: Bytes) -> Result<Vec<D>>;

    async fn fetch(&self, params: Value, creds: &Credentials) -> Result<OBBject<D>> {
        let query = Self::transform_query(params)?;
        let raw = self.extract_data(&query, creds).await?;
        let rows = self.transform_data(&query, raw)?;
        Ok(OBBject::new(rows, Self::PROVIDER, Self::ENDPOINT))
    }
}

pub type DataStream<T> = Pin<Box<dyn Stream<Item = Result<T>> + Send>>;

#[async_trait]
pub trait Streamer<Q, D>: Send + Sync + 'static
where
    Q: QueryParams,
    D: DataModel,
{
    const PROVIDER: &'static str;
    const ENDPOINT: &'static str;

    async fn subscribe(&self, query: Q, creds: &Credentials) -> Result<DataStream<D>>;
    async fn snapshot(&self, query: &Q, creds: &Credentials) -> Result<Vec<D>>;
    async fn checkpoint(&self, _seq: u64) -> Result<()> {
        Ok(())
    }
}

pub type ProgressStream<T> = Pin<Box<dyn Stream<Item = Result<ProgressOrResult<T>>> + Send>>;

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LexicalDoc {
    pub id: String,
    pub body: String,
    pub fields: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
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

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn register_fetcher<F, Q, D>(&mut self) -> Result<()>
    where
        F: Fetcher<Q, D>,
        Q: QueryParams,
        D: DataModel,
    {
        self.register(RegistryEntry::fetcher(F::PROVIDER, F::ENDPOINT))
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn register_streamer<S, Q, D>(&mut self) -> Result<()>
    where
        S: Streamer<Q, D>,
        Q: QueryParams,
        D: DataModel,
    {
        self.register(RegistryEntry::streamer(S::PROVIDER, S::ENDPOINT))
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub fn from_inventory() -> Result<Self> {
        let registry = Self::default();
        #[cfg(feature = "inventory-registration")]
        {
            return Self::with_inventory_entries(registry);
        }
        #[cfg(not(feature = "inventory-registration"))]
        {
            Ok(registry)
        }
    }

    #[cfg(feature = "inventory-registration")]
    fn with_inventory_entries(mut registry: Self) -> Result<Self> {
        for entry in inventory::iter::<RegistryEntry> {
            registry.register(entry.clone())?;
        }
        Ok(registry)
    }

    #[must_use]
    pub fn resolve(
        &self,
        provider: &str,
        endpoint: &str,
        kind: ProviderKind,
    ) -> Option<&RegistryEntry> {
        self.entries.iter().find(|entry| {
            entry.provider == provider && entry.endpoint == endpoint && entry.kind == kind
        })
    }

    #[must_use]
    pub fn contains(&self, provider: &str, endpoint: &str, kind: ProviderKind) -> bool {
        self.resolve(provider, endpoint, kind).is_some()
    }

    #[must_use]
    pub fn entries(&self) -> &[RegistryEntry] {
        &self.entries
    }
}

impl RegistryEntry {
    #[must_use]
    pub const fn fetcher(provider: &'static str, endpoint: &'static str) -> Self {
        Self {
            provider,
            endpoint,
            kind: ProviderKind::Fetcher,
        }
    }

    #[must_use]
    pub const fn streamer(provider: &'static str, endpoint: &'static str) -> Self {
        Self {
            provider,
            endpoint,
            kind: ProviderKind::Streamer,
        }
    }
}

#[cfg(feature = "inventory-registration")]
inventory::collect!(RegistryEntry);

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use serde_json::json;

    #[test]
    fn safe_sql_identifier_accepts_valid_names_and_rejects_injection() {
        for ok in [
            "tdw_outbox",
            "tdw_sessions",
            "_private",
            "schema.table",
            "a1.b2.c3",
            "T",
        ] {
            assert!(safe_sql_identifier(ok).is_ok(), "{ok:?} should be valid");
        }
        for bad in [
            "",                       // empty
            "1bad",                   // leading digit
            "x; drop table sessions", // statement injection
            "x\" or \"1\"=\"1",       // quote injection
            "x--comment",             // comment injection
            "has space",              // whitespace
            ".leading",               // leading dot
            "trailing.",              // trailing dot
            "a..b",                   // doubled dot
            "naïve",                  // non-ASCII
        ] {
            assert!(
                safe_sql_identifier(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
        // Length is bounded.
        assert!(safe_sql_identifier(&"a".repeat(64)).is_ok());
        assert!(safe_sql_identifier(&"a".repeat(65)).is_err());
    }

    #[test]
    fn safe_path_segment_accepts_filenames_and_rejects_traversal() {
        for ok in ["buf", "note", "research.note", "a_b-c", ".hidden"] {
            assert!(safe_path_segment(ok).is_ok(), "{ok:?} should be valid");
        }
        for bad in [
            "",            // empty
            "..",          // parent
            ".",           // current
            "a/b",         // separator (subdir)
            "a\\b",        // windows separator
            "../escape",   // traversal
            "/etc/shadow", // absolute
            "a\0b",        // NUL / control
        ] {
            assert!(
                safe_path_segment(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn safe_relative_path_allows_subdirs_and_rejects_absolute_and_traversal() {
        for ok in ["prompts/foo.txt", "a/b/c", "single.txt"] {
            assert!(safe_relative_path(ok).is_ok(), "{ok:?} should be valid");
        }
        for bad in [
            "",            // empty
            "  ",          // blank
            "../secret",   // traversal
            "a/../b",      // embedded traversal
            "/etc/shadow", // absolute (the HK3 bypass)
            "./relative",  // current-dir prefix
            "a\\b",        // backslash
            "line\nbreak", // control char
        ] {
            assert!(
                safe_relative_path(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

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

        fn transform_query(params: Value) -> Result<Query> {
            let symbol = params
                .get("symbol")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::InvalidQuery("missing symbol".to_string()))?;
            Ok(Query {
                symbol: symbol.to_string(),
            })
        }

        async fn extract_data(&self, query: &Query, _creds: &Credentials) -> Result<Bytes> {
            Ok(Bytes::from(query.symbol.clone()))
        }

        fn transform_data(&self, _query: &Query, raw: Bytes) -> Result<Vec<Row>> {
            let symbol = String::from_utf8(raw.to_vec())
                .map_err(|error| Error::Provider(error.to_string()))?;
            Ok(vec![Row { symbol }])
        }
    }

    struct MockStreamer;

    #[async_trait]
    impl Streamer<Query, Row> for MockStreamer {
        const PROVIDER: &'static str = "mock-ws";
        const ENDPOINT: &'static str = "equity_ticks";

        async fn subscribe(&self, _query: Query, _creds: &Credentials) -> Result<DataStream<Row>> {
            Ok(Box::pin(EmptyStream))
        }

        async fn snapshot(&self, query: &Query, _creds: &Credentials) -> Result<Vec<Row>> {
            Ok(vec![Row {
                symbol: query.symbol.clone(),
            }])
        }
    }

    struct EmptyStream;

    impl Stream for EmptyStream {
        type Item = Result<Row>;

        fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(None)
        }
    }

    #[cfg(feature = "inventory-registration")]
    inventory::submit! {
        RegistryEntry::fetcher("inventory", "equity_historical")
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
    fn envelope_round_trips_json() {
        let object = OBBject::new(
            vec![Row {
                symbol: "MSFT".to_string(),
            }],
            "fileset",
            "equity_historical",
        )
        .with_metadata("source", json!("golden"));

        let json = serde_json::to_string(&object)
            .unwrap_or_else(|error| panic!("object should serialize: {error}"));
        let round_trip: OBBject<Row> = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("object should deserialize: {error}"));

        assert_eq!(round_trip, object);
    }

    #[test]
    fn registry_rejects_duplicate_entries() {
        let mut registry = ProviderRegistry::default();
        let entry = RegistryEntry::fetcher("fileset", "equity_historical");

        assert!(registry.register(entry.clone()).is_ok());
        let duplicate = registry.register(entry);

        assert!(duplicate.is_err());
    }

    #[test]
    fn registry_registers_fetchers_and_streamers_explicitly() {
        let mut registry = ProviderRegistry::default();

        registry
            .register_fetcher::<MockFetcher, Query, Row>()
            .unwrap_or_else(|error| panic!("mock fetcher should register: {error}"));
        registry
            .register_streamer::<MockStreamer, Query, Row>()
            .unwrap_or_else(|error| panic!("mock streamer should register: {error}"));

        assert!(registry.contains("mock", "equity_historical", ProviderKind::Fetcher));
        assert!(registry.contains("mock-ws", "equity_ticks", ProviderKind::Streamer));
    }

    #[test]
    fn registry_allows_distinct_provider_endpoint_and_kind_combinations() {
        let mut registry = ProviderRegistry::default();

        for entry in [
            RegistryEntry::fetcher("mock", "equity_historical"),
            RegistryEntry::fetcher("mock", "quotes"),
            RegistryEntry::fetcher("fileset", "equity_historical"),
            RegistryEntry::streamer("mock", "equity_historical"),
        ] {
            registry
                .register(entry)
                .unwrap_or_else(|error| panic!("distinct entry should register: {error}"));
        }

        assert_eq!(registry.entries().len(), 4);
        assert!(
            registry
                .resolve("mock", "equity_historical", ProviderKind::Fetcher)
                .is_some()
        );
        assert!(
            registry
                .resolve("mock", "equity_historical", ProviderKind::Streamer)
                .is_some()
        );
        assert!(
            registry
                .resolve("mock", "missing", ProviderKind::Fetcher)
                .is_none()
        );
        assert!(
            registry
                .resolve("missing", "equity_historical", ProviderKind::Fetcher)
                .is_none()
        );
        assert!(!registry.contains("mock", "missing", ProviderKind::Fetcher));
        assert_eq!(registry.entries()[3].kind, ProviderKind::Streamer);
    }

    #[cfg(feature = "inventory-registration")]
    #[test]
    fn registry_loads_inventory_entries_when_feature_enabled() {
        let registry = ProviderRegistry::from_inventory()
            .unwrap_or_else(|error| panic!("inventory registry should load: {error}"));

        assert!(registry.contains("inventory", "equity_historical", ProviderKind::Fetcher));
    }
}
