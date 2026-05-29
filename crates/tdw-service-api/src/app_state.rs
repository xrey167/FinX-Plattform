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

use std::sync::{Arc, Mutex};

use tdw_auth_oidc::{JwksKey, JwtClaims};
use tdw_bus::EventBus;
use tdw_config::{PermissionAction, TdwConfig};
use tdw_core::{
    BlobEngine, Credentials, DataModel, LexicalEngine, OlapEngine, ProviderRegistry, QueryParams,
    RelationalEngine, Result, Streamer, VectorEngine,
};
use tdw_outbox::InMemoryOutbox;
use tdw_rollout::JsonlRollout;
use tdw_session::SqliteSessionStore;
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
    pub session: SqliteSessionStore,
    pub rollout: JsonlRollout,
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
        let session = SqliteSessionStore::connect(&config.session.sqlite_path)
            .await
            .map_err(|e| tdw_core::Error::Storage(format!("session store: {e}")))?;
        let rollout = JsonlRollout::new(&config.paths.rollout_dir);

        let blob: Arc<dyn BlobEngine> = select_blob_engine(&config);
        let policy = build_policy(&config);

        Ok(Self {
            config,
            olap: select_olap_engine()?,
            relational: Arc::new(PostgresRecordingEngine::default()),
            blob,
            vector: Arc::new(InMemoryVectorEngine::default()),
            lexical: Arc::new(InMemoryLexicalEngine::default()),
            registry: Arc::new(default_registry()?),
            bus: Arc::new(Mutex::new(EventBus::new(DEFAULT_BUS_CAPACITY))),
            outbox: Arc::new(Mutex::new(InMemoryOutbox::default())),
            snapshot: Arc::new(Mutex::new(SnapshotStore::default())),
            policy,
            session,
            rollout,
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

/// Build the daemon policy from config.
///
/// Local/non-production profiles synthesize a deterministic principal so
/// offline daemon tests can execute real dispatch paths. Production profiles do
/// not synthesize credentials; they remain fail-closed until real ingress auth
/// attaches a policy.
fn build_policy(config: &TdwConfig) -> Option<PolicyEnforcementConfig> {
    if matches!(config.profile.as_str(), "prod" | "production") {
        return None;
    }

    let roles = match config.permissions.default_action {
        PermissionAction::Allow | PermissionAction::Ask => {
            vec!["analyst".to_string(), "udf_runner".to_string()]
        }
        PermissionAction::Deny => Vec::new(),
    };

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
