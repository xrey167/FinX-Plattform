//! Daemon composition root (P0 of the integration cycle).
//!
//! `AppState` owns the selected engines, the provider registry, the event-spine
//! stores, and the policy enforcement configuration. `from_config` builds an
//! `AppState` with deterministic in-memory backends; feature-gated real engines
//! (`sqlx_engine` / `http_engine` / `aws_engine`) hook in here in later phases
//! by branching on `TdwConfig` without changing the daemon callers.

use std::sync::{Arc, Mutex};

use tdw_bus::EventBus;
use tdw_config::TdwConfig;
use tdw_core::{
    BlobEngine, LexicalEngine, OlapEngine, ProviderRegistry, RelationalEngine, Result, VectorEngine,
};
use tdw_outbox::InMemoryOutbox;
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
}

impl AppState {
    /// Build an in-memory `AppState` from a layered `TdwConfig`.
    pub fn from_config(config: TdwConfig) -> Result<Self> {
        Ok(Self {
            config,
            olap: Arc::new(ClickHouseRecordingEngine::default()),
            relational: Arc::new(PostgresRecordingEngine::default()),
            blob: Arc::new(InMemoryS3BlobEngine::default()),
            vector: Arc::new(InMemoryVectorEngine::default()),
            lexical: Arc::new(InMemoryLexicalEngine::default()),
            registry: Arc::new(default_registry()?),
            bus: Arc::new(Mutex::new(EventBus::new(DEFAULT_BUS_CAPACITY))),
            outbox: Arc::new(Mutex::new(InMemoryOutbox::default())),
            snapshot: Arc::new(Mutex::new(SnapshotStore::default())),
            policy: None,
        })
    }

    pub fn with_policy(mut self, policy: PolicyEnforcementConfig) -> Self {
        self.policy = Some(policy);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_core::ProviderKind;

    #[test]
    fn builds_in_memory_app_state_from_default_config() {
        let state = AppState::from_config(TdwConfig::default())
            .unwrap_or_else(|error| panic!("AppState should build from default config: {error}"));

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

    #[test]
    fn carries_layered_config_profile_into_app_state() {
        let config = TdwConfig {
            profile: "service".to_string(),
            ..Default::default()
        };
        let state = AppState::from_config(config)
            .unwrap_or_else(|error| panic!("AppState should build: {error}"));
        assert_eq!(state.config.profile, "service");
    }
}
