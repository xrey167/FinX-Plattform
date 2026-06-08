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

#[cfg(feature = "compaction")]
pub mod compaction;
#[cfg(feature = "http")]
pub mod http_support;
pub mod turn;

pub mod query_params;

pub use query_params::{Date, Interval, MAX_LIMIT, Period, StandardParams};

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

/// Generate the canonical base-URL-only HTTP fetcher scaffolding.
///
/// Many `tdw-provider-*` HTTP fetchers share an identical shape: a struct with
/// a single `base_url: String` field, a [`Default`] impl seeded from the
/// provider's `BASE_URL` constant, a `with_base_url` builder, a private
/// `base_url()` accessor, and a `registry_entry()` helper. This macro expands
/// to exactly that scaffolding so each provider only needs to write its
/// per-provider [`Fetcher`] impl (`transform_query`/`extract_data`/
/// `transform_data`).
///
/// The macro references [`RegistryEntry`] via `$crate`, so callers need no
/// extra import. `Self::PROVIDER`/`Self::ENDPOINT` in the generated
/// `registry_entry()` resolve against the [`Fetcher`] impl the provider writes
/// separately, so this macro must be invoked in the same module as that impl.
///
/// # Example
///
/// ```ignore
/// const BASE_URL: &str = "https://api.example.com";
/// tdw_core::provider_fetcher_struct!(pub ExampleHttpFetcher, BASE_URL);
/// // ... then write `impl Fetcher<Q, D> for ExampleHttpFetcher { ... }`
/// // and read the URL inside `extract_data` via `self.base_url()`.
/// ```
#[macro_export]
macro_rules! provider_fetcher_struct {
    ($(#[$meta:meta])* $vis:vis $name:ident, $base_url:expr $(,)?) => {
        $(#[$meta])*
        #[derive(Clone, Debug)]
        $vis struct $name {
            base_url: String,
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    base_url: $base_url.to_string(),
                }
            }
        }

        impl $name {
            /// Override the base URL (useful for testing against a mock server).
            #[must_use]
            pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
                self.base_url = base_url.into();
                self
            }

            /// Registry entry advertised under this provider's canonical name.
            #[must_use]
            pub fn registry_entry() -> $crate::RegistryEntry {
                $crate::RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)
            }

            #[allow(dead_code)]
            pub(crate) fn base_url(&self) -> &str {
                &self.base_url
            }
        }
    };
}

#[cfg(feature = "inventory-registration")]
inventory::collect!(RegistryEntry);

/// Shared, dependency-free date/time conversion primitives.
///
/// These helpers centralise the verbatim Howard-Hinnant `civil_from_days`
/// algorithm and the ISO-8601 timestamp/date formatting tails that were
/// previously copy-pasted across several `tdw-provider-*` crates. The math is
/// reproduced character-for-character so emitted strings remain byte-identical
/// to the former inlined implementations.
pub mod date {
    /// Convert a count of days since the Unix epoch (1970-01-01) into a
    /// `(year, month, day)` civil date using the Howard-Hinnant algorithm.
    #[must_use]
    // month is 1..=12 and day is 1..=31 by the Howard-Hinnant algorithm, so the
    // i64->u32 casts cannot truncate or lose sign. const fn cannot use try_from,
    // and the math must stay byte-identical to the former inlined versions.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub const fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
        let days = days_since_epoch + 719_468;
        let era = days.div_euclid(146_097);
        let doe = days - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let mut year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        if month <= 2 {
            year += 1;
        }
        (year, month as u32, day as u32)
    }

    /// Convert a whole-second Unix timestamp to an ISO-8601 UTC timestamp
    /// string of the form `YYYY-MM-DDThh:mm:ssZ`.
    #[must_use]
    pub fn unix_seconds_to_iso_timestamp(seconds: i64) -> String {
        let days_since_epoch = seconds.div_euclid(86_400);
        let seconds_of_day = seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days_since_epoch);
        let hour = seconds_of_day / 3_600;
        let minute = (seconds_of_day % 3_600) / 60;
        let second = seconds_of_day % 60;
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    }

    /// Convert a whole-second Unix timestamp to an ISO-8601 UTC date string of
    /// the form `YYYY-MM-DD`.
    #[must_use]
    pub fn unix_seconds_to_iso_date(seconds: i64) -> String {
        let days_since_epoch = seconds.div_euclid(86_400);
        let (year, month, day) = civil_from_days(days_since_epoch);
        format!("{year:04}-{month:02}-{day:02}")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn civil_from_days_well_known() {
            assert_eq!(civil_from_days(0), (1970, 1, 1));
            assert_eq!(civil_from_days(1_704_153_600 / 86_400), (2024, 1, 2));
            assert_eq!(
                civil_from_days((-86_400i64).div_euclid(86_400)),
                (1969, 12, 31)
            );
        }

        #[test]
        fn unix_seconds_to_iso_timestamp_well_known() {
            assert_eq!(unix_seconds_to_iso_timestamp(0), "1970-01-01T00:00:00Z");
            assert_eq!(
                unix_seconds_to_iso_timestamp(1_704_153_600),
                "2024-01-02T00:00:00Z"
            );
            assert_eq!(
                unix_seconds_to_iso_timestamp(-86_400),
                "1969-12-31T00:00:00Z"
            );
        }

        #[test]
        fn unix_seconds_to_iso_date_well_known() {
            assert_eq!(unix_seconds_to_iso_date(0), "1970-01-01");
            assert_eq!(unix_seconds_to_iso_date(1_704_153_600), "2024-01-02");
            assert_eq!(unix_seconds_to_iso_date(-86_400), "1969-12-31");
        }
    }
}
