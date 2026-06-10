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

#[cfg(feature = "compaction")]
pub mod compaction;
pub mod graph;
#[cfg(feature = "http")]
pub mod http_support;
pub mod turn;

pub mod query_params;

pub use graph::{
    Direction, GraphEdge, GraphEngine, GraphNode, MAX_HOPS, MergeDecision, MergeReport, Provenance,
    Subgraph, TraversalFilter, active_at, is_graph_id, validate_graph_edge, validate_graph_node,
    validate_traversal_filter,
};
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
    /// Enumerate stored documents — the durable-document scan behind
    /// `tdw kg reindex` (knowledge-system B6). Order is engine-defined but
    /// STABLE across pages while the index is unmodified, so
    /// `offset`/`limit` pagination covers every document exactly once.
    /// `limit` must be greater than zero; an offset at/past the end returns
    /// an empty page.
    async fn documents(&self, index: &str, offset: usize, limit: usize) -> Result<Vec<LexicalDoc>>;
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

/// One payload/fields condition (knowledge-system B1).
///
/// `MatchString` against an array-valued field means "array contains value" —
/// exactly Qdrant's `match` semantics on array fields, mirrored by the in-memory
/// engines so backends cannot drift.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PayloadCondition {
    /// `payload[key] == value`, or `payload[key]` is an array containing `value`.
    MatchString { key: String, value: String },
    /// `MatchString` against any of `values`.
    MatchAny { key: String, values: Vec<String> },
    /// Inclusive string range over a string field. Used for `as_of` bounds; the
    /// field must hold normalized RFC 3339 UTC strings so lexicographic order is
    /// chronological (the Qdrant backend maps this to a datetime range filter).
    ///
    /// Lexical backends may reject this condition loudly: Meilisearch comparison
    /// filters are numeric-only, so temporal range filtering belongs on the
    /// vector channel (the in-memory lexical engine supports it for tests).
    ///
    /// # Temporal convention
    ///
    /// Both bounds are **inclusive** (`[gte, lte]`, a closed interval) — matching
    /// Qdrant's range semantics. This DIFFERS from the graph layer's half-open
    /// `[valid_from, valid_to)` windows: when deriving a `RangeString` from a
    /// graph validity window, do NOT pass the exclusive `valid_to` directly as
    /// `lte`, or documents stamped exactly at the boundary leak in.
    RangeString {
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gte: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lte: Option<String>,
    },
}

/// Conjunctive payload filter applied at search time. Empty = match everything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadFilter {
    /// All conditions must hold.
    #[serde(default)]
    pub must: Vec<PayloadCondition>,
}

impl PayloadFilter {
    /// Whether the filter has no conditions (matches everything).
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.must.is_empty()
    }

    /// Evaluate against a payload/fields object. This is the SHARED reference
    /// evaluator used by every in-memory engine, so filtered search behaves
    /// identically across modalities by construction.
    #[must_use]
    pub fn matches(&self, payload: &Value) -> bool {
        self.must.iter().all(|condition| condition.matches(payload))
    }
}

impl PayloadCondition {
    /// Evaluate one condition against a payload/fields object.
    #[must_use]
    pub fn matches(&self, payload: &Value) -> bool {
        match self {
            Self::MatchString { key, value } => string_field_matches(payload, key, value),
            Self::MatchAny { key, values } => values
                .iter()
                .any(|value| string_field_matches(payload, key, value)),
            Self::RangeString { key, gte, lte } => {
                payload.get(key).and_then(Value::as_str).is_some_and(|s| {
                    gte.as_deref().is_none_or(|bound| s >= bound)
                        && lte.as_deref().is_none_or(|bound| s <= bound)
                })
            }
        }
    }
}

fn string_field_matches(payload: &Value, key: &str, value: &str) -> bool {
    match payload.get(key) {
        Some(Value::String(field)) => field == value,
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| item.as_str().is_some_and(|field| field == value)),
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorQuery {
    pub vector: Vec<f32>,
    pub top_k: usize,
    /// Payload filter applied before scoring; empty = unfiltered. The pre-B1 wire
    /// shape is preserved in BOTH directions: old JSON deserializes (`default`)
    /// and an unfiltered query serializes without the field (`skip_serializing_if`).
    #[serde(default, skip_serializing_if = "PayloadFilter::is_empty")]
    pub filter: PayloadFilter,
}

impl VectorQuery {
    /// An unfiltered KNN query — the pre-B1 construction.
    #[must_use]
    pub fn knn(vector: Vec<f32>, top_k: usize) -> Self {
        Self {
            vector,
            top_k,
            filter: PayloadFilter::default(),
        }
    }
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
    /// Fields filter applied before scoring; empty = unfiltered. The pre-B1 wire
    /// shape is preserved in BOTH directions: old JSON deserializes (`default`)
    /// and an unfiltered query serializes without the field (`skip_serializing_if`).
    #[serde(default, skip_serializing_if = "PayloadFilter::is_empty")]
    pub filter: PayloadFilter,
}

impl TextQuery {
    /// An unfiltered text query — the pre-B1 construction.
    #[must_use]
    pub fn text(text: impl Into<String>, top_k: usize) -> Self {
        Self {
            text: text.into(),
            top_k,
            filter: PayloadFilter::default(),
        }
    }
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
            Self::with_inventory_entries(registry)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use serde_json::json;

    #[test]
    fn payload_filter_matches_strings_arrays_ranges_and_conjunction() {
        let payload = json!({
            "entity_id": "instrument:AAPL",
            "tags": ["asset:equity", "region:us"],
            "as_of": "2026-03-01T00:00:00Z",
            "rank": 3,
        });

        // Empty filter matches everything (the pre-B1 behavior).
        assert!(PayloadFilter::default().matches(&payload));
        assert!(PayloadFilter::default().is_empty());

        let match_string = |key: &str, value: &str| PayloadCondition::MatchString {
            key: key.to_string(),
            value: value.to_string(),
        };
        // String equality, array-contains, and misses (incl. non-string fields).
        assert!(match_string("entity_id", "instrument:AAPL").matches(&payload));
        assert!(match_string("tags", "asset:equity").matches(&payload));
        assert!(!match_string("tags", "asset:bond").matches(&payload));
        assert!(!match_string("missing", "x").matches(&payload));
        assert!(!match_string("rank", "3").matches(&payload));

        // MatchAny is any-of over the same per-value rule.
        assert!(
            PayloadCondition::MatchAny {
                key: "tags".to_string(),
                values: vec!["asset:bond".to_string(), "region:us".to_string()],
            }
            .matches(&payload)
        );

        // Inclusive string range; open bounds pass; non-string fields never match.
        let range = |gte: Option<&str>, lte: Option<&str>| PayloadCondition::RangeString {
            key: "as_of".to_string(),
            gte: gte.map(ToString::to_string),
            lte: lte.map(ToString::to_string),
        };
        assert!(range(None, Some("2026-12-31T00:00:00Z")).matches(&payload));
        assert!(range(Some("2026-03-01T00:00:00Z"), None).matches(&payload));
        assert!(!range(Some("2026-06-01T00:00:00Z"), None).matches(&payload));

        // Conjunction: all must hold.
        let filter = PayloadFilter {
            must: vec![
                match_string("tags", "asset:equity"),
                range(None, Some("2026-01-01T00:00:00Z")),
            ],
        };
        assert!(!filter.matches(&payload));
    }

    #[test]
    fn query_constructors_build_unfiltered_queries_and_serde_defaults_filter() {
        let knn = VectorQuery::knn(vec![1.0], 5);
        assert!(knn.filter.is_empty());
        let text = TextQuery::text("volatility", 5);
        assert!(text.filter.is_empty());
        // The pre-B1 wire shape holds in the SERIALIZE direction too: an
        // unfiltered query emits no `filter` key at all.
        let encoded = serde_json::to_value(&knn).expect("vector query should serialize");
        assert!(encoded.get("filter").is_none());
        let encoded = serde_json::to_value(&text).expect("text query should serialize");
        assert!(encoded.get("filter").is_none());
        // Pre-B1 wire shapes still deserialize (filter defaults to empty).
        let decoded: VectorQuery = serde_json::from_value(json!({ "vector": [1.0], "top_k": 5 }))
            .expect("pre-B1 vector query should deserialize");
        assert!(decoded.filter.is_empty());
        let decoded: TextQuery =
            serde_json::from_value(json!({ "text": "volatility", "top_k": 5 }))
                .expect("pre-B1 text query should deserialize");
        assert!(decoded.filter.is_empty());
    }

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
