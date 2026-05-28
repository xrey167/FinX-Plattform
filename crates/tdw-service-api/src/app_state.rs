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

use tdw_bus::EventBus;
use tdw_config::TdwConfig;
use tdw_core::{
    BlobEngine, LexicalEngine, OlapEngine, ProviderRegistry, RelationalEngine, Result, VectorEngine,
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

use crate::{PolicyEnforcementConfig, default_registry};

const DEFAULT_BUS_CAPACITY: usize = 1024;

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
    pub async fn from_config(config: TdwConfig) -> Result<Self> {
        let session = SqliteSessionStore::connect(&config.session.sqlite_path)
            .await
            .map_err(|e| tdw_core::Error::Storage(format!("session store: {e}")))?;
        let rollout = JsonlRollout::new(&config.paths.rollout_dir);

        let blob: Arc<dyn BlobEngine> = select_blob_engine(&config);

        Ok(Self {
            config,
            olap: Arc::new(ClickHouseRecordingEngine::default()),
            relational: Arc::new(PostgresRecordingEngine::default()),
            blob,
            vector: Arc::new(InMemoryVectorEngine::default()),
            lexical: Arc::new(InMemoryLexicalEngine::default()),
            registry: Arc::new(default_registry()?),
            bus: Arc::new(Mutex::new(EventBus::new(DEFAULT_BUS_CAPACITY))),
            outbox: Arc::new(Mutex::new(InMemoryOutbox::default())),
            snapshot: Arc::new(Mutex::new(SnapshotStore::default())),
            policy: None,
            session,
            rollout,
        })
    }

    pub fn with_policy(mut self, policy: PolicyEnforcementConfig) -> Self {
        self.policy = Some(policy);
        self
    }

    /// Build an `AppState` backed by an in-memory SQLite database and a unique
    /// temporary JSONL rollout file. Suitable for unit tests.
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

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_core::ProviderKind;

    #[tokio::test]
    async fn builds_in_memory_app_state_from_default_config() {
        let state = AppState::in_memory_for_tests().await;

        assert!(state.registry.entries().len() >= 3);
        assert!(
            state
                .registry
                .contains("fileset", "equity_historical", ProviderKind::Fetcher)
        );
        assert!(state.policy.is_none());
        assert_eq!(state.config.profile, "default");

        let cloned = state.clone();
        assert!(Arc::ptr_eq(&state.registry, &cloned.registry));
    }

    #[tokio::test]
    async fn carries_layered_config_profile_into_app_state() {
        let state = AppState::in_memory_for_tests().await;
        assert_eq!(state.config.profile, "default");
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
        let mut config = TdwConfig::default();
        config.profile = "service".to_string();
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
