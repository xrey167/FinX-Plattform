//! Daemon composition root (P0 of the integration cycle).
//!
//! `AppState` owns the selected engines, the provider registry, the event-spine
//! stores, and the policy enforcement configuration. `from_config` builds an
//! `AppState` with deterministic in-memory backends; feature-gated real engines
//! (`sqlx_engine` / `http_engine` / `aws_engine`) hook in here in later phases
//! by branching on `TdwConfig` without changing the daemon callers.
//!
//! # Adapter registration pattern (P5)
//!
//! See `docs/ADAPTER_PATTERN.md` for the full 3-step recipe. In brief:
//!
//! 1. **Implement the trait** in an adapter crate (e.g. `BlobEngine` in
//!    `tdw-storage-fs`, or `SandboxRuntime`-compatible logic in `tdw-udf-wasm`).
//! 2. **Feature-gate the live path** — add an optional dep + feature on
//!    `tdw-service-api` (`storage-fs`, `udf-wasm`, …). The in-memory default
//!    remains active when the feature is absent.
//! 3. **Register** in `AppState::from_config` (engines) or via
//!    `default_registry` / sandbox routing (providers + UDF runtimes).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tdw_app_server::CancellationToken;
use tdw_auth_oidc::{DEFAULT_ALLOWED_ALGORITHMS, JwksKey, JwtClaims, validate_claims_strict};
use tdw_bus::EventBus;
use tdw_config::{PermissionAction, TdwConfig};
use tdw_core::{
    BlobEngine, Credentials, DataModel, Error, LexicalEngine, OlapEngine, ProviderRegistry,
    QueryParams, RelationalEngine, Result, Streamer, VectorEngine,
};
use tdw_outbox::InMemoryOutbox;
use tdw_protocol::SessionId;
use tdw_rollout::{JsonlRollout, RolloutRecord};
use tdw_session::{CostLedgerEntry, SessionError, SessionRecord, SqliteSessionStore};
use tdw_snapshot::SnapshotStore;
use tdw_storage_clickhouse::ClickHouseRecordingEngine;
use tdw_storage_meilisearch::InMemoryLexicalEngine;
use tdw_storage_postgres::PostgresRecordingEngine;
use tdw_storage_qdrant::InMemoryVectorEngine;
use tdw_storage_s3::InMemoryS3BlobEngine;

use crate::{IngressAuthContext, PolicyEnforcementConfig, default_registry, run_ws_ingest};

const DEFAULT_BUS_CAPACITY: usize = 1024;
const LOCAL_POLICY_ISSUER: &str = "tdw://local-dev";
const LOCAL_POLICY_AUDIENCE: &str = "tdw-daemon";
const LOCAL_POLICY_KID: &str = "local-dev";

/// Daemon session store backend.
///
/// `Sqlite` is the always-available default (in-memory or file). `Pg` is
/// selected at runtime in the `live` stack when the `daemon-postgres` feature
/// is built and a Postgres URL is configured, so the cost ledger / session rows
/// survive container restarts. Only the methods the daemon actually drives
/// (`upsert_session`, `append_cost`, `cost_entries`) are exposed; both arms are
/// unified onto [`tdw_core::Error`].
#[derive(Clone)]
pub enum SessionBackend {
    Sqlite(SqliteSessionStore),
    #[cfg(feature = "daemon-postgres")]
    Pg(tdw_session::PgSessionStore),
}

impl SessionBackend {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn upsert_session(&self, record: &SessionRecord) -> Result<()> {
        match self {
            Self::Sqlite(store) => store
                .upsert_session(record)
                .await
                .map_err(session_storage_err),
            #[cfg(feature = "daemon-postgres")]
            Self::Pg(store) => store.upsert_session(record).await,
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn append_cost(&self, entry: &CostLedgerEntry) -> Result<()> {
        match self {
            Self::Sqlite(store) => store.append_cost(entry).await.map_err(session_storage_err),
            #[cfg(feature = "daemon-postgres")]
            Self::Pg(store) => store.append_cost(entry).await,
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn cost_entries(&self, session_id: &SessionId) -> Result<Vec<CostLedgerEntry>> {
        match self {
            Self::Sqlite(store) => store
                .cost_entries(session_id)
                .await
                .map_err(session_storage_err),
            #[cfg(feature = "daemon-postgres")]
            Self::Pg(store) => store.cost_entries(session_id).await,
        }
    }
}

/// Daemon rollout (replay/audit) backend.
///
/// `Jsonl` is the always-available default (a local JSONL file). `Pg` mirrors
/// it into a Postgres table when the `daemon-postgres` feature is built and a
/// Postgres URL is configured, so rollout survives container restarts. Methods
/// are async (the `Pg` arm awaits the DB; the `Jsonl` arm is sync underneath).
#[derive(Clone)]
pub enum RolloutBackend {
    Jsonl(JsonlRollout),
    #[cfg(feature = "daemon-postgres")]
    Pg(tdw_rollout::PgRollout),
}

impl RolloutBackend {
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn append(&self, record: &RolloutRecord) -> Result<()> {
        match self {
            Self::Jsonl(rollout) => rollout
                .append(record)
                .map_err(|error| Error::Storage(error.to_string())),
            #[cfg(feature = "daemon-postgres")]
            Self::Pg(rollout) => rollout.append(record).await,
        }
    }

    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn read_all(&self) -> Result<Vec<RolloutRecord>> {
        match self {
            Self::Jsonl(rollout) => rollout
                .read_all()
                .map_err(|error| Error::Storage(error.to_string())),
            #[cfg(feature = "daemon-postgres")]
            Self::Pg(rollout) => rollout.read_all().await,
        }
    }
}

/// Map a `tdw-session` error onto the unified [`tdw_core::Error`].
fn session_storage_err(error: SessionError) -> Error {
    Error::Storage(error.to_string())
}

/// Handle to a running streaming-ingest task.
///
/// Holds the [`CancellationToken`] that stops the background `run_ws_ingest`
/// loop and the [`JoinHandle`] resolving to the total rows written. Stored in
/// [`AppState::streams`] keyed by `stream_id`.
///
/// [`JoinHandle`]: tokio::task::JoinHandle
pub struct StreamControl {
    /// Cancels the background ingest task when triggered.
    pub cancel: CancellationToken,
    /// Resolves to the total rows written (or `Ok(0)` on cancellation).
    pub handle: tokio::task::JoinHandle<Result<usize>>,
}

/// Composition root for the daemon.
#[derive(Clone)]
pub struct AppState {
    pub config: TdwConfig,
    pub olap: Arc<dyn OlapEngine>,
    pub relational: Arc<dyn RelationalEngine>,
    pub blob: Arc<dyn BlobEngine>,
    pub vector: Arc<dyn VectorEngine>,
    pub lexical: Arc<dyn LexicalEngine>,
    pub registry: Arc<ProviderRegistry>,
    pub bus: Arc<Mutex<EventBus>>,
    pub outbox: Arc<Mutex<InMemoryOutbox>>,
    pub snapshot: Arc<Mutex<SnapshotStore>>,
    pub policy: Option<PolicyEnforcementConfig>,
    /// In a `prod`/`production` boot, the specific reason no auth-backed policy
    /// was attached when the OIDC config was *partially* present or invalid.
    /// `None` for a fully-unset (intentional fail-closed) prod boot and for all
    /// non-prod profiles (which synthesize a local policy). Read at boot to log
    /// the actionable cause; see `tdw-service/src/main.rs`.
    pub policy_attach_error: Option<OidcPolicyError>,
    pub session: SessionBackend,
    pub rollout: RolloutBackend,
    /// Live streaming-ingest tasks keyed by `stream_id`. Shared by reference
    /// across `AppState` clones (the daemon clones `AppState` per connection),
    /// so a stream started on one clone is visible to `stop_stream` on another.
    pub streams: Arc<Mutex<HashMap<String, StreamControl>>>,
}

impl AppState {
    /// Build an `AppState` from a layered `TdwConfig`.
    ///
    /// Engine selection follows the adapter pattern documented in
    /// `docs/ADAPTER_PATTERN.md`:
    ///
    /// - **blob**: `InMemoryS3BlobEngine` by default; `LocalFsBlobEngine` when
    ///   the `storage-fs` feature is enabled **and** `config.profile == "service"`.
    /// - **udf/wasm**: routing is handled inside `tdw-sandbox` (enabled by the
    ///   `udf-wasm` feature on this crate which is forwarded to `tdw-sandbox`).
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    pub async fn from_config(config: TdwConfig) -> Result<Self> {
        let (session, rollout) = build_durable_stores(&config).await?;

        let blob: Arc<dyn BlobEngine> = select_blob_engine(&config);
        let (policy, policy_attach_error) = build_policy(&config);

        Ok(Self {
            config,
            olap: select_olap_engine()?,
            relational: Arc::new(PostgresRecordingEngine::default()),
            blob,
            vector: select_vector_engine()?,
            lexical: Arc::new(InMemoryLexicalEngine::default()),
            registry: Arc::new(default_registry()?),
            bus: Arc::new(Mutex::new(EventBus::new(DEFAULT_BUS_CAPACITY))),
            outbox: Arc::new(Mutex::new(InMemoryOutbox::default())),
            snapshot: Arc::new(Mutex::new(SnapshotStore::default())),
            policy,
            policy_attach_error,
            session,
            rollout,
            streams: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[must_use]
    pub fn with_policy(mut self, policy: PolicyEnforcementConfig) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Replace the relational engine with a real [`PgEngine`] connected to
    /// `database_url`.
    ///
    /// Only available when the `real-postgres` feature is enabled (which
    /// activates `tdw-storage-postgres/postgres`). The method is deliberately
    /// absent on default builds so CI without Docker still compiles cleanly.
    ///
    /// Typical test usage:
    /// ```ignore
    /// let state = AppState::in_memory_for_tests()
    ///     .await
    ///     .with_policy(analyst_policy())
    ///     .with_real_postgres(&url)
    ///     .await
    ///     .expect("pg connect");
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error variant if the underlying operation fails.
    #[cfg(feature = "real-postgres")]
    pub async fn with_real_postgres(mut self, database_url: &str) -> tdw_core::Result<Self> {
        let engine = tdw_storage_postgres::PgEngine::connect(database_url)
            .await
            .map_err(|e| tdw_core::Error::Storage(format!("pg connect: {e}")))?;
        self.relational = std::sync::Arc::new(engine);
        Ok(self)
    }

    /// Replace the OLAP engine with a real [`ClickHouseHttpEngine`] talking to
    /// `endpoint` (e.g. `http://127.0.0.1:8123`) over the ClickHouse HTTP
    /// interface. This is what makes the ingest path (`dispatch_ingest` /
    /// `run_stream_ingest`, which call `olap.execute(INSERT … FORMAT JSONEachRow)`)
    /// and the query path land on a live ClickHouse instead of the offline
    /// recording engine.
    ///
    /// Only available when the `real-clickhouse` feature is enabled (which
    /// activates `tdw-storage-clickhouse/clickhouse`); the method is absent on
    /// default builds so the offline workspace test set still compiles cleanly.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the endpoint URL cannot be parsed.
    #[cfg(feature = "real-clickhouse")]
    pub fn with_real_clickhouse(
        mut self,
        endpoint: &str,
        user: Option<String>,
        password: Option<String>,
    ) -> Result<Self> {
        let engine = tdw_storage_clickhouse::ClickHouseHttpEngine::new(endpoint, user, password)?;
        self.olap = std::sync::Arc::new(engine);
        Ok(self)
    }

    /// Replace the vector engine with a real [`QdrantHttpEngine`] talking to
    /// `endpoint` (e.g. `http://127.0.0.1:6333`) over the Qdrant HTTP
    /// interface. This is what makes the knowledge index (built over
    /// `AppState.vector`) land on a live Qdrant instead of the in-memory
    /// engine, so indexed knowledge persists across restarts.
    ///
    /// Only available when the `real-qdrant` feature is enabled (which activates
    /// `tdw-storage-qdrant/qdrant`); the method is absent on default builds so
    /// the offline workspace test set still compiles cleanly.
    ///
    /// # Errors
    ///
    /// Returns an error variant if the endpoint URL cannot be parsed.
    #[cfg(feature = "real-qdrant")]
    pub fn with_real_qdrant(mut self, endpoint: &str, api_key: Option<String>) -> Result<Self> {
        let engine = tdw_storage_qdrant::QdrantHttpEngine::new(endpoint, api_key)?;
        self.vector = std::sync::Arc::new(engine);
        Ok(self)
    }

    /// Build an `AppState` backed by an in-memory `SQLite` database and a unique
    /// temporary JSONL rollout file. Suitable for unit tests.
    ///
    /// # Panics
    ///
    /// Panics if the system clock is before the Unix epoch, or if building the
    /// in-memory `AppState` fails.
    pub async fn in_memory_for_tests() -> Self {
        let mut config = TdwConfig::default();
        config.session.sqlite_path = "sqlite::memory:".to_string();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        config.paths.rollout_dir = std::env::temp_dir()
            .join(format!("tdw-rollout-{nanos}.jsonl"))
            .to_string_lossy()
            .into_owned();
        Self::from_config(config)
            .await
            .unwrap_or_else(|e| panic!("in_memory_for_tests should build AppState: {e}"))
    }

    /// Start a background streaming ingest task that drains a [`Streamer`]
    /// subscription into the OLAP engine, racing the ingest against the supplied
    /// [`CancellationToken`].
    ///
    /// This is the daemon-facing entrypoint: it clones the shared `olap` Arc,
    /// derives a stable `session_id` of `stream:{PROVIDER}:{ENDPOINT}`, and
    /// `tokio::spawn`s a task running [`run_ws_ingest`]. The returned
    /// [`JoinHandle`] resolves to the total rows written (or `Ok(0)` if the
    /// token cancels first).
    ///
    /// On cancellation the in-flight (unflushed) buffer is dropped, so its rows
    /// are not written. This is safe: each flushed batch carries a
    /// content-addressed `insert_deduplication_token` (a hash of the batch
    /// scoped to `session_id` + `table`), so re-ingesting the same source after
    /// a restart deduplicates identical batches at the `ClickHouse` INSERT and
    /// does not double-count through the materialized views. The pipeline is
    /// therefore at-least-once: a dropped tail is re-fetched on restart, and
    /// any already-persisted batch dedups instead of duplicating.
    ///
    /// [`JoinHandle`]: tokio::task::JoinHandle
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn spawn_stream_ingest<S, Q, D>(
        &self,
        streamer: S,
        query: Q,
        creds: Credentials,
        table: String,
        max_rows: usize,
        max_wait: std::time::Duration,
        cancel: tdw_app_server::CancellationToken,
    ) -> tokio::task::JoinHandle<tdw_core::Result<usize>>
    where
        S: Streamer<Q, D>,
        Q: QueryParams,
        D: DataModel,
    {
        let olap = Arc::clone(&self.olap);
        let session_id = format!("stream:{}:{}", S::PROVIDER, S::ENDPOINT);
        tokio::spawn(async move {
            tokio::select! {
                res = run_ws_ingest(
                    olap.as_ref(),
                    &streamer,
                    query,
                    &creds,
                    &session_id,
                    &table,
                    max_rows,
                    max_wait,
                ) => res,
                () = cancel.cancelled() => Ok(0),
            }
        })
    }

    /// Start a live Binance trade streaming-ingest task for `symbol`, draining
    /// the websocket subscription into `table` (defaulting to `raw.tick`) via
    /// [`spawn_stream_ingest`](Self::spawn_stream_ingest).
    ///
    /// The symbol is normalized (uppercased, validated) before use. A fresh
    /// [`CancellationToken`] is created and the resulting [`StreamControl`] is
    /// registered in [`streams`](Self::streams) under a stable
    /// `stream_id` of `binance:trades:{SYMBOL}`, which is returned. With the
    /// `ws` feature the task connects to the public Binance trade stream;
    /// without it the deterministic offline streamer is used.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Provider`] if the symbol is invalid, if the internal
    /// streams lock is poisoned, or if a stream with the same `stream_id` is
    /// already running (not yet finished).
    pub fn start_binance_stream(&self, symbol: &str, table: Option<String>) -> Result<String> {
        use tdw_provider_binance::{BinanceTradeQuery, BinanceTradeStreamer};

        let query = BinanceTradeQuery::new(symbol)
            .map_err(|error| Error::Provider(format!("binance stream symbol: {error}")))?;
        let stream_id = format!("binance:trades:{}", query.symbol);
        let table = table.unwrap_or_else(|| "raw.tick".to_string());

        {
            let streams = self
                .streams
                .lock()
                .map_err(|_| Error::Provider("streams registry lock poisoned".to_string()))?;
            if streams
                .get(&stream_id)
                .is_some_and(|control| !control.handle.is_finished())
            {
                return Err(Error::Provider(format!(
                    "stream already running: {stream_id}"
                )));
            }
        }

        let cancel = CancellationToken::new();
        let handle = self.spawn_stream_ingest(
            BinanceTradeStreamer,
            query,
            Credentials::default(),
            table,
            500,
            std::time::Duration::from_millis(500),
            cancel.clone(),
        );

        let mut streams = self
            .streams
            .lock()
            .map_err(|_| Error::Provider("streams registry lock poisoned".to_string()))?;
        streams.insert(stream_id.clone(), StreamControl { cancel, handle });
        Ok(stream_id)
    }

    /// Stop a running streaming-ingest task identified by `stream_id`.
    ///
    /// Cancels the task's [`CancellationToken`] and removes its
    /// [`StreamControl`] from [`streams`](Self::streams). The entry is taken out
    /// of the map and the lock guard dropped before cancelling, so no `await`
    /// or task interaction happens while holding the mutex.
    ///
    /// Returns `true` if a stream with that id was present, `false` otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Provider`] if the internal streams lock is poisoned.
    pub fn stop_stream(&self, stream_id: &str) -> Result<bool> {
        let control = {
            let mut streams = self
                .streams
                .lock()
                .map_err(|_| Error::Provider("streams registry lock poisoned".to_string()))?;
            streams.remove(stream_id)
        };
        match control {
            Some(control) => {
                control.cancel.cancel();
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Select the blob engine based on compile-time features and runtime config.
///
/// Step 3 of the adapter pattern: register in `AppState::from_config`.
fn select_blob_engine(config: &TdwConfig) -> Arc<dyn BlobEngine> {
    // When the `storage-fs` feature is compiled in and the profile is
    // "service", use the local filesystem blob engine rooted at `data_dir`.
    #[cfg(feature = "storage-fs")]
    if config.profile == "service" {
        use tdw_storage_fs::LocalBlobEngine;
        return Arc::new(LocalBlobEngine::new(&config.paths.data_dir));
    }

    // Suppress unused-variable warning on the non-feature path.
    let _ = config;
    Arc::new(InMemoryS3BlobEngine::default())
}

/// Select the OLAP engine. With the `real-clickhouse` feature enabled and
/// `TDW_CLICKHOUSE_URL` set, the daemon talks to a live `ClickHouse` over HTTP
/// (so the ingest/query path persists for real); otherwise the offline
/// `ClickHouseRecordingEngine` is used. `TDW_CLICKHOUSE_USER` /
/// `TDW_CLICKHOUSE_PASSWORD` are optional (default user, no password).
///
/// # Errors
///
/// Returns an error if `TDW_CLICKHOUSE_URL` is set but cannot be parsed.
// Real Err path exists behind `real-clickhouse` (ClickHouseHttpEngine::new(..)?);
// only looks unwrappable in the default build, so keep the Result and the allow.
#[allow(clippy::unnecessary_wraps)]
fn select_olap_engine() -> Result<Arc<dyn OlapEngine>> {
    #[cfg(feature = "real-clickhouse")]
    {
        if let Ok(url) = std::env::var("TDW_CLICKHOUSE_URL")
            && !url.trim().is_empty()
        {
            let user = std::env::var("TDW_CLICKHOUSE_USER").ok();
            let password = std::env::var("TDW_CLICKHOUSE_PASSWORD").ok();
            let engine = tdw_storage_clickhouse::ClickHouseHttpEngine::new(&url, user, password)?;
            return Ok(Arc::new(engine));
        }
    }
    Ok(Arc::new(ClickHouseRecordingEngine::default()))
}

/// Select the vector engine. With the `real-qdrant` feature enabled and
/// `TDW_QDRANT_URL` set, the daemon talks to a live Qdrant over HTTP (so the
/// knowledge index built over `AppState.vector` persists for real); otherwise
/// the offline `InMemoryVectorEngine` is used. `TDW_QDRANT_API_KEY` is optional
/// (default: no API key).
///
/// # Errors
///
/// Returns an error if `TDW_QDRANT_URL` is set but cannot be parsed.
// Real Err path exists behind `real-qdrant` (QdrantHttpEngine::new(..)?); only
// looks unwrappable in the default build, so keep the Result and the allow.
#[allow(clippy::unnecessary_wraps)]
fn select_vector_engine() -> Result<Arc<dyn VectorEngine>> {
    #[cfg(feature = "real-qdrant")]
    {
        if let Ok(url) = std::env::var("TDW_QDRANT_URL")
            && !url.trim().is_empty()
        {
            let api_key = std::env::var("TDW_QDRANT_API_KEY").ok();
            let engine = tdw_storage_qdrant::QdrantHttpEngine::new(&url, api_key)?;
            return Ok(Arc::new(engine));
        }
    }
    Ok(Arc::new(InMemoryVectorEngine::default()))
}

/// Build the daemon's session + rollout stores.
///
/// Default (and the only path without the `daemon-postgres` feature): a SQLite
/// session store and a JSONL rollout file, exactly as before. With
/// `daemon-postgres` built **and** a Postgres URL configured (`TDW_DAEMON_PG_URL`
/// or `DATABASE_URL`), both stores are Postgres-backed instead so they survive
/// container restarts in the `live` stack; their schemas are created on connect.
async fn build_durable_stores(config: &TdwConfig) -> Result<(SessionBackend, RolloutBackend)> {
    #[cfg(feature = "daemon-postgres")]
    if let Some(url) = daemon_pg_url() {
        let engine = tdw_storage_postgres::PgEngine::connect(&url)
            .await
            .map_err(|e| Error::Storage(format!("daemon pg connect: {e}")))?;

        let session = tdw_session::PgSessionStore::new(engine.clone());
        session.ensure_schema().await?;

        let rollout =
            tdw_rollout::PgRollout::new(Arc::new(engine) as Arc<dyn tdw_core::RelationalEngine>);
        rollout.ensure_schema().await?;

        return Ok((SessionBackend::Pg(session), RolloutBackend::Pg(rollout)));
    }

    let session = SqliteSessionStore::connect(&config.session.sqlite_path)
        .await
        .map_err(|e| Error::Storage(format!("session store: {e}")))?;
    let rollout = JsonlRollout::new(&config.paths.rollout_dir);
    Ok((
        SessionBackend::Sqlite(session),
        RolloutBackend::Jsonl(rollout),
    ))
}

/// Resolve the daemon's own Postgres URL from the environment, preferring the
/// dedicated `TDW_DAEMON_PG_URL` over a shared `DATABASE_URL`. Returns `None`
/// when neither is set (non-empty), keeping the SQLite/JSONL default.
#[cfg(feature = "daemon-postgres")]
fn daemon_pg_url() -> Option<String> {
    ["TDW_DAEMON_PG_URL", "DATABASE_URL"]
        .into_iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
}

/// Typed failure cause for production OIDC policy construction.
///
/// Each variant names a specific, operator-actionable reason the auth-backed
/// production policy could not be attached, so a misconfigured `prod`/`production`
/// boot can log *why* it fell back to fail-closed (rather than collapsing every
/// cause to a bare `None`). `InvalidClaims` carries the rich
/// [`tdw_auth_oidc::ClaimValidationError`] from structural validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OidcPolicyError {
    /// A required `TDW_OIDC_*` variable was unset or blank (the named var).
    MissingEnvVar(&'static str),
    /// A `kid:alg` JWKS segment was malformed (missing `:` or an empty side).
    MalformedJwksPair(String),
    /// The same `kid` appeared more than once in the JWKS spec.
    DuplicateKid(String),
    /// Structural claim/JWKS validation rejected the inputs.
    InvalidClaims(tdw_auth_oidc::ClaimValidationError),
}

impl std::fmt::Display for OidcPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEnvVar(var) => write!(f, "{var} missing"),
            Self::MalformedJwksPair(pair) => write!(f, "malformed JWKS pair: {pair}"),
            Self::DuplicateKid(kid) => write!(f, "duplicate kid in JWKS: {kid}"),
            Self::InvalidClaims(error) => write!(f, "invalid claims: {error:?}"),
        }
    }
}

/// Build the daemon policy from config.
///
/// Local/non-production profiles synthesize a deterministic principal so
/// offline daemon tests can execute real dispatch paths. Production profiles do
/// not synthesize credentials; they remain fail-closed until real ingress auth
/// attaches a policy. Any production attach failure is returned alongside the
/// (absent) policy so the caller can surface the specific cause at boot.
fn build_policy(config: &TdwConfig) -> (Option<PolicyEnforcementConfig>, Option<OidcPolicyError>) {
    // TDW_DAEMON_OPEN_POLICY=1 opts in to the local-dev policy regardless of
    // TDW_PROFILE, for operator environments that don't yet have OIDC wired.
    if std::env::var("TDW_DAEMON_OPEN_POLICY").as_deref() == Ok("1") {
        return build_local_policy(config);
    }
    if matches!(config.profile.as_str(), "prod" | "production") {
        return match build_prod_policy_from_env() {
            Ok(policy) => (policy, None),
            Err(error) => (None, Some(error)),
        };
    }
    build_local_policy(config)
}

/// Synthesize the local-dev policy from `config`. Used by non-prod profiles
/// and by the `TDW_DAEMON_OPEN_POLICY=1` escape hatch.
fn build_local_policy(
    config: &TdwConfig,
) -> (Option<PolicyEnforcementConfig>, Option<OidcPolicyError>) {
    let roles = match config.permissions.default_action {
        PermissionAction::Allow | PermissionAction::Ask => {
            vec!["analyst".to_string(), "udf_runner".to_string()]
        }
        PermissionAction::Deny => Vec::new(),
    };

    (
        Some(PolicyEnforcementConfig {
            auth: IngressAuthContext {
                claims: JwtClaims {
                    sub: "local:default".to_string(),
                    iss: LOCAL_POLICY_ISSUER.to_string(),
                    aud: LOCAL_POLICY_AUDIENCE.to_string(),
                    kid: LOCAL_POLICY_KID.to_string(),
                    roles,
                },
                jwks: vec![JwksKey {
                    kid: LOCAL_POLICY_KID.to_string(),
                    alg: "RS256".to_string(),
                }],
                issuer: LOCAL_POLICY_ISSUER.to_string(),
                audience: LOCAL_POLICY_AUDIENCE.to_string(),
            },
            hooks: Vec::new(),
            hook_execution: tdw_hooks::HookExecutionPolicy::default(),
            mask_rules: Vec::new(),
        }),
        None,
    )
}

/// Read a required, non-blank environment variable. Returns `None` when the
/// variable is unset or trims to empty, so callers fail closed.
fn required_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The five required `TDW_OIDC_*` variables, in the order they are read.
const REQUIRED_OIDC_VARS: [&str; 5] = [
    "TDW_OIDC_ISSUER",
    "TDW_OIDC_AUDIENCE",
    "TDW_OIDC_JWKS",
    "TDW_OIDC_SUBJECT",
    "TDW_OIDC_KID",
];

/// Production policy wrapper: reads the five required `TDW_OIDC_*` variables via
/// [`required_env`] (plus the optional `TDW_OIDC_ROLES`) and delegates to
/// [`build_prod_policy_from_lookup`].
///
/// See [`build_prod_policy_from_lookup`] for the all-unset/partial/valid
/// semantics.
///
/// # Errors
///
/// Returns [`OidcPolicyError`] when some (but not all) required variables are
/// set, or when the OIDC inputs fail structural validation.
fn build_prod_policy_from_env()
-> std::result::Result<Option<PolicyEnforcementConfig>, OidcPolicyError> {
    build_prod_policy_from_lookup(required_env)
}

/// Env-injectable production policy construction.
///
/// `lookup` returns the trimmed, non-blank value of a `TDW_OIDC_*` variable (or
/// `None` when unset/blank). Taking a closure keeps this race-free and testable
/// without mutating the process environment.
///
/// Semantics:
/// - **all five required vars unset → `Ok(None)`** (deliberately unconfigured,
///   fail-closed by design — not an error).
/// - **some required vars present, others missing → `Err(MissingEnvVar(..))`**
///   naming the first missing required var (a partial misconfiguration).
/// - **all required present but parse/validation fails → `Err(..)`**.
/// - **all valid → `Ok(Some(policy))`**.
///
/// `TDW_OIDC_ROLES` is optional and never triggers the missing-var path.
///
/// # Errors
///
/// Returns [`OidcPolicyError`] for a partial configuration or any
/// parse/validation failure (see semantics above).
fn build_prod_policy_from_lookup(
    lookup: impl Fn(&str) -> Option<String>,
) -> std::result::Result<Option<PolicyEnforcementConfig>, OidcPolicyError> {
    let resolved: Vec<Option<String>> = REQUIRED_OIDC_VARS.iter().map(|key| lookup(key)).collect();

    // All five required vars unset: intentionally unconfigured, fail closed.
    if resolved.iter().all(Option::is_none) {
        return Ok(None);
    }

    // Some present, some missing: a partial misconfiguration is an error so the
    // operator sees which var to set rather than a silent fail-closed.
    let mut required = Vec::with_capacity(REQUIRED_OIDC_VARS.len());
    for (key, value) in REQUIRED_OIDC_VARS.iter().zip(resolved) {
        match value {
            Some(value) => required.push(value),
            None => return Err(OidcPolicyError::MissingEnvVar(key)),
        }
    }

    let roles_spec = lookup("TDW_OIDC_ROLES").unwrap_or_default();
    build_prod_policy(
        &required[0],
        &required[1],
        &required[2],
        &required[3],
        &required[4],
        &roles_spec,
    )
    .map(Some)
}

/// Pure (env-free) construction of an auth-backed production policy.
///
/// Parses `jwks_spec` (comma-separated `kid:alg` pairs) and `roles_spec`
/// (comma-separated role names), assembles the principal claims, and validates
/// the result with [`validate_claims_strict`] against
/// [`DEFAULT_ALLOWED_ALGORITHMS`]. This checks claim/JWKS structural
/// consistency only — it does not verify cryptographic signatures.
///
/// Returns an [`OidcPolicyError`] (fail closed) when the JWKS spec is
/// malformed/empty or validation rejects the inputs (e.g. unknown kid,
/// unsupported algorithm, invalid role). Empty/whitespace role entries are
/// skipped.
///
/// # Errors
///
/// - [`OidcPolicyError::MalformedJwksPair`] when a `kid:alg` segment is missing
///   its `:` separator, has an empty side, or the whole spec is empty/blank.
/// - [`OidcPolicyError::DuplicateKid`] when a `kid` appears more than once.
/// - [`OidcPolicyError::InvalidClaims`] when structural validation fails.
fn build_prod_policy(
    issuer: &str,
    audience: &str,
    jwks_spec: &str,
    subject: &str,
    kid: &str,
    roles_spec: &str,
) -> std::result::Result<PolicyEnforcementConfig, OidcPolicyError> {
    let mut jwks = Vec::new();
    for pair in jwks_spec.split(',') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        let (key_id, alg) = pair
            .split_once(':')
            .ok_or_else(|| OidcPolicyError::MalformedJwksPair(pair.to_string()))?;
        let key_id = key_id.trim();
        let alg = alg.trim();
        if key_id.is_empty() || alg.is_empty() {
            return Err(OidcPolicyError::MalformedJwksPair(pair.to_string()));
        }
        jwks.push(JwksKey {
            kid: key_id.to_string(),
            alg: alg.to_string(),
        });
    }
    if jwks.is_empty() {
        return Err(OidcPolicyError::MalformedJwksPair(jwks_spec.to_string()));
    }
    let mut seen = std::collections::HashSet::new();
    if let Some(duplicate) = jwks.iter().find(|k| !seen.insert(k.kid.as_str())) {
        return Err(OidcPolicyError::DuplicateKid(duplicate.kid.clone()));
    }

    let roles = roles_spec
        .split(',')
        .map(str::trim)
        .filter(|role| !role.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    let claims = JwtClaims {
        sub: subject.to_string(),
        iss: issuer.to_string(),
        aud: audience.to_string(),
        kid: kid.to_string(),
        roles,
    };

    validate_claims_strict(
        &claims,
        &jwks,
        issuer,
        audience,
        &DEFAULT_ALLOWED_ALGORITHMS,
    )
    .map_err(OidcPolicyError::InvalidClaims)?;

    Ok(PolicyEnforcementConfig {
        auth: IngressAuthContext {
            claims,
            jwks,
            issuer: issuer.to_string(),
            audience: audience.to_string(),
        },
        hooks: Vec::new(),
        hook_execution: tdw_hooks::HookExecutionPolicy::default(),
        mask_rules: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_app_server::CancellationToken;
    use tdw_core::ProviderKind;
    use tdw_provider_ws::{WsTickQuery, WsTickStreamer};

    #[tokio::test]
    async fn builds_in_memory_app_state_from_default_config() {
        let state = AppState::in_memory_for_tests().await;

        assert!(state.registry.entries().len() >= 3);
        assert!(
            state
                .registry
                .contains("fileset", "equity_historical", ProviderKind::Fetcher)
        );
        let policy = state.policy.as_ref().expect("local policy");
        assert_eq!(policy.auth.claims.sub, "local:default");
        assert!(
            policy
                .auth
                .claims
                .roles
                .iter()
                .any(|role| role == "analyst")
        );
        assert_eq!(state.config.profile, "default");

        let cloned = state.clone();
        assert!(Arc::ptr_eq(&state.registry, &cloned.registry));
    }

    fn aapl_ws_query() -> WsTickQuery {
        WsTickQuery {
            url: String::new(),
            symbol: "AAPL".to_string(),
        }
    }

    #[tokio::test]
    async fn spawn_stream_ingest_persists_offline_stream() {
        let state = AppState::in_memory_for_tests().await;
        let cancel = CancellationToken::new();

        let handle = state.spawn_stream_ingest(
            WsTickStreamer,
            aapl_ws_query(),
            Credentials::default(),
            "raw.tick".to_string(),
            10,
            std::time::Duration::from_millis(50),
            cancel,
        );

        let total = handle
            .await
            .unwrap_or_else(|e| panic!("ingest task should not panic: {e}"))
            .unwrap_or_else(|e| panic!("ingest should succeed: {e}"));
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn spawn_stream_ingest_completes_cleanly_when_cancelled_first() {
        let state = AppState::in_memory_for_tests().await;
        let cancel = CancellationToken::new();
        // Cancel before spawning: the select! must resolve to Ok(_) without hanging.
        cancel.cancel();

        let handle = state.spawn_stream_ingest(
            WsTickStreamer,
            aapl_ws_query(),
            Credentials::default(),
            "raw.tick".to_string(),
            10,
            std::time::Duration::from_millis(50),
            cancel,
        );

        let result = handle
            .await
            .unwrap_or_else(|e| panic!("ingest task should not panic: {e}"));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn carries_layered_config_profile_into_app_state() {
        let state = AppState::in_memory_for_tests().await;
        assert_eq!(state.config.profile, "default");
    }

    #[tokio::test]
    async fn deny_permissions_build_local_fail_closed_policy() {
        let mut config = TdwConfig::default();
        config.session.sqlite_path = "sqlite::memory:".to_string();
        config.permissions.default_action = PermissionAction::Deny;

        let state = AppState::from_config(config)
            .await
            .unwrap_or_else(|e| panic!("AppState should build: {e}"));

        let roles = &state.policy.expect("local policy").auth.claims.roles;
        assert!(roles.is_empty());
    }

    #[tokio::test]
    async fn docker_profile_attaches_local_policy_so_live_dispatches_resolve() {
        // The compose `live` stack runs the daemon with `TDW_PROFILE: docker`.
        // `docker` is non-prod, so `build_policy` must synthesize a local policy;
        // without one the dispatcher returns Failed for every op.
        let mut config = TdwConfig {
            profile: "docker".to_string(),
            ..TdwConfig::default()
        };
        config.session.sqlite_path = "sqlite::memory:".to_string();

        let state = AppState::from_config(config)
            .await
            .unwrap_or_else(|e| panic!("AppState should build: {e}"));

        let policy = state
            .policy
            .expect("docker profile must attach a local policy");
        assert_eq!(policy.auth.claims.sub, "local:default");
        assert!(
            policy
                .auth
                .claims
                .roles
                .iter()
                .any(|role| role == "analyst")
        );
    }

    #[tokio::test]
    async fn production_profile_does_not_synthesize_local_policy() {
        let mut config = TdwConfig {
            profile: "production".to_string(),
            ..TdwConfig::default()
        };
        config.session.sqlite_path = "sqlite::memory:".to_string();

        let state = AppState::from_config(config)
            .await
            .unwrap_or_else(|e| panic!("AppState should build: {e}"));

        assert!(state.policy.is_none());
    }

    #[test]
    fn build_prod_policy_accepts_valid_oidc_inputs() {
        let policy = build_prod_policy(
            "https://issuer.example",
            "tdw-daemon",
            "key1:RS256,key2:ES256",
            "svc:prod",
            "key1",
            "analyst,udf_runner",
        )
        .expect("valid OIDC inputs should yield a policy");

        assert_eq!(policy.auth.claims.sub, "svc:prod");
        assert_eq!(policy.auth.issuer, "https://issuer.example");
        assert_eq!(policy.auth.audience, "tdw-daemon");
        assert_eq!(policy.auth.claims.kid, "key1");
        assert_eq!(
            policy.auth.claims.roles,
            vec!["analyst".to_string(), "udf_runner".to_string()]
        );
        assert!(policy.auth.jwks.iter().any(|key| key.kid == "key1"));
        assert!(policy.hooks.is_empty());
        assert!(policy.mask_rules.is_empty());
    }

    #[test]
    fn build_prod_policy_rejects_blank_required_fields() {
        // Blank subject → claim validation rejects an empty `sub`.
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                "key1:RS256",
                "   ",
                "key1",
                "",
            ),
            Err(OidcPolicyError::InvalidClaims(
                tdw_auth_oidc::ClaimValidationError::EmptySubject
            ))
        );
        // Empty JWKS spec → malformed (no usable pairs); carries the raw spec.
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                "",
                "svc:prod",
                "key1",
                "",
            ),
            Err(OidcPolicyError::MalformedJwksPair(String::new()))
        );
        // Blank issuer → claim validation rejects an issuer mismatch/empty.
        assert_eq!(
            build_prod_policy("   ", "tdw-daemon", "key1:RS256", "svc:prod", "key1", ""),
            Err(OidcPolicyError::InvalidClaims(
                tdw_auth_oidc::ClaimValidationError::IssuerMismatch
            ))
        );
    }

    #[test]
    fn build_prod_policy_rejects_kid_not_in_jwks() {
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                "key1:RS256,key2:ES256",
                "svc:prod",
                "key3",
                "",
            ),
            Err(OidcPolicyError::InvalidClaims(
                tdw_auth_oidc::ClaimValidationError::UnknownKeyId
            ))
        );
    }

    #[test]
    fn build_prod_policy_rejects_unsupported_algorithm() {
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                "key1:none",
                "svc:prod",
                "key1",
                "",
            ),
            Err(OidcPolicyError::InvalidClaims(
                tdw_auth_oidc::ClaimValidationError::UnsupportedAlgorithm
            ))
        );
    }

    #[test]
    fn build_prod_policy_rejects_malformed_jwks_pair() {
        // Missing the `:alg` separator → carries the offending segment.
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                "key1",
                "svc:prod",
                "key1",
                "",
            ),
            Err(OidcPolicyError::MalformedJwksPair("key1".to_string()))
        );
        // Empty alg side → still malformed, carries the offending segment.
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                "key1:",
                "svc:prod",
                "key1",
                "",
            ),
            Err(OidcPolicyError::MalformedJwksPair("key1:".to_string()))
        );
    }

    #[test]
    fn build_prod_policy_parses_roles() {
        let two = build_prod_policy(
            "https://issuer.example",
            "tdw-daemon",
            "key1:RS256",
            "svc:prod",
            "key1",
            "analyst,udf_runner",
        )
        .expect("valid inputs");
        assert_eq!(two.auth.claims.roles.len(), 2);

        let none = build_prod_policy(
            "https://issuer.example",
            "tdw-daemon",
            "key1:RS256",
            "svc:prod",
            "key1",
            "",
        )
        .expect("valid inputs with no roles");
        assert!(none.auth.claims.roles.is_empty());
    }

    #[test]
    fn build_prod_policy_rejects_duplicate_kids_in_jwks() {
        // Same kid with different algs — the second entry would shadow the first;
        // reject to prevent ambiguous key resolution.
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                "key1:RS256,key1:ES256",
                "svc:prod",
                "key1",
                "",
            ),
            Err(OidcPolicyError::DuplicateKid("key1".to_string()))
        );
    }

    #[test]
    fn build_prod_policy_rejects_whitespace_only_jwks_spec() {
        // A spec of only delimiters/whitespace segments must be fail-closed;
        // no usable pairs → malformed (carries the raw spec).
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                ",, ,",
                "svc:prod",
                "key1",
                "",
            ),
            Err(OidcPolicyError::MalformedJwksPair(",, ,".to_string()))
        );
    }

    #[test]
    fn build_prod_policy_rejects_role_with_invalid_characters() {
        // A space inside a role name is invalid per the role-name grammar.
        assert_eq!(
            build_prod_policy(
                "https://issuer.example",
                "tdw-daemon",
                "key1:RS256",
                "svc:prod",
                "key1",
                "analyst,bad role",
            ),
            Err(OidcPolicyError::InvalidClaims(
                tdw_auth_oidc::ClaimValidationError::InvalidRole
            ))
        );
    }

    /// Exact `valid_oidc_inputs` set used by the wrapper + E2E tests.
    fn valid_oidc_lookup(key: &str) -> Option<String> {
        match key {
            "TDW_OIDC_ISSUER" => Some("https://issuer.example".to_string()),
            "TDW_OIDC_AUDIENCE" => Some("tdw-daemon".to_string()),
            "TDW_OIDC_JWKS" => Some("key1:RS256,key2:ES256".to_string()),
            "TDW_OIDC_SUBJECT" => Some("svc:prod".to_string()),
            "TDW_OIDC_KID" => Some("key1".to_string()),
            "TDW_OIDC_ROLES" => Some("analyst,udf_runner".to_string()),
            _ => None,
        }
    }

    #[test]
    fn build_prod_policy_from_lookup_all_unset_is_ok_none() {
        // No TDW_OIDC_* present at all: a deliberately unconfigured prod boot is
        // fail-closed by design, not an error.
        let result = build_prod_policy_from_lookup(|_| None).expect("all-unset must be Ok(None)");
        assert!(result.is_none());
    }

    #[test]
    fn build_prod_policy_from_lookup_partial_config_names_missing_var() {
        // Issuer + audience present, JWKS (a required var) missing: the wrapper
        // must surface the first missing required var, not silently fail closed.
        let map = std::collections::HashMap::from([
            ("TDW_OIDC_ISSUER", "https://issuer.example"),
            ("TDW_OIDC_AUDIENCE", "tdw-daemon"),
        ]);
        let result = build_prod_policy_from_lookup(|key| map.get(key).map(ToString::to_string));
        assert_eq!(result, Err(OidcPolicyError::MissingEnvVar("TDW_OIDC_JWKS")));
    }

    #[test]
    fn build_prod_policy_from_lookup_all_valid_is_ok_some() {
        let policy = build_prod_policy_from_lookup(valid_oidc_lookup)
            .expect("all-valid must be Ok")
            .expect("all-valid must yield Some(policy)");
        assert_eq!(policy.auth.claims.sub, "svc:prod");
        assert_eq!(policy.auth.issuer, "https://issuer.example");
        assert_eq!(
            policy.auth.claims.roles,
            vec!["analyst".to_string(), "udf_runner".to_string()]
        );
    }

    /// Build the same valid production policy the wrapper would, directly via
    /// the pure fn (no env), so the E2E tests stay race-free in the parallel
    /// suite. `analyst` is included so the `RunQuery` (analyst-gated) endpoint
    /// authorizes.
    fn prod_built_policy() -> PolicyEnforcementConfig {
        build_prod_policy(
            "https://issuer.example",
            "tdw-daemon",
            "key1:RS256,key2:ES256",
            "svc:prod",
            "key1",
            "analyst,udf_runner",
        )
        .expect("valid prod OIDC inputs should yield a policy")
    }

    #[tokio::test]
    async fn prod_built_policy_lets_dispatch_resolve_to_completed() {
        use tdw_protocol::{ActorKind, ActorRef, EventMsg, Op, OpEnvelope, SessionId};

        // The HIGH gap: prove a *prod-built* policy actually resolves a dispatch
        // (not just the in-memory `policy_config` ones in the unit suite).
        let state = AppState::in_memory_for_tests()
            .await
            .with_policy(prod_built_policy());

        let env = OpEnvelope::new(
            SessionId::new("session-prod-e2e").expect("session id"),
            1,
            ActorRef {
                actor_id: "user:test".to_string(),
                kind: ActorKind::User,
                tenant_id: Some("default".to_string()),
            },
            Op::RunQuery {
                sql: "select 1".to_string(),
                plan_id: None,
                cost_hint: None,
            },
        );
        let op_id = env.op_id.clone();
        let events = crate::dispatch_op(&state, env).await;

        assert_eq!(events.len(), 2);
        match &events[0] {
            EventMsg::Started { op_id: started } => assert_eq!(started, &op_id),
            other => panic!("expected Started, got {other:?}"),
        }
        match &events[1] {
            EventMsg::Completed {
                op_id: completed, ..
            } => assert_eq!(completed, &op_id),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn prod_built_policy_rejects_invalid_ingress_claims() {
        // The request-time `validate_claims` gate must also hold for prod-built
        // policies: a tampered ingress claim (audience mismatch) is rejected
        // before any provider/UDF work runs.
        let mut policy = prod_built_policy();
        policy.auth.claims.aud = "attacker-audience".to_string();

        let mut backend = tdw_hooks::SystemHookHandlerBackend::default();
        let rejected =
            crate::secure_endpoint_response_with_backend(&policy, "fileset", "aapl", &mut backend)
                .expect_err("tampered ingress claims must be rejected");
        assert!(
            rejected.to_string().contains("ingress jwt rejected"),
            "expected ingress rejection, got: {rejected}"
        );
    }

    /// With `storage-fs` feature: "default" profile must still use the
    /// in-memory engine (Arc downcast is not stable, so we just check it builds).
    #[tokio::test]
    async fn default_profile_always_uses_in_memory_blob() {
        let state = AppState::in_memory_for_tests().await;
        // profile == "default" → in-memory path regardless of features.
        assert_eq!(state.config.profile, "default");
        // Engine is present and usable (Arc is not None).
        let _ = Arc::clone(&state.blob);
    }

    /// With `storage-fs` feature: "service" profile selects `LocalFsBlobEngine`.
    #[cfg(feature = "storage-fs")]
    #[tokio::test]
    async fn service_profile_selects_fs_blob_engine() {
        let mut config = TdwConfig {
            profile: "service".to_string(),
            ..TdwConfig::default()
        };
        config.session.sqlite_path = "sqlite::memory:".to_string();
        config.paths.data_dir = std::env::temp_dir()
            .join("tdw-blob-test")
            .to_string_lossy()
            .into_owned();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        config.paths.rollout_dir = std::env::temp_dir()
            .join(format!("tdw-rollout-{nanos}.jsonl"))
            .to_string_lossy()
            .into_owned();

        let state = AppState::from_config(config)
            .await
            .unwrap_or_else(|e| panic!("service AppState should build: {e}"));

        assert_eq!(state.config.profile, "service");
        // Engine is present and usable.
        let _ = Arc::clone(&state.blob);
    }
}
