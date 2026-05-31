//! Async data/daemon facade.
//!
//! Phase 0 wires construction only: it owns the daemon composition root
//! ([`AppState`]) and a [`CommandRunner`] for provider dispatch. Later phases add
//! the query/ingest methods on top of these fields.

use tdw_config::TdwConfig;
use tdw_runtime::CommandRunner;
use tdw_service_api::AppState;

use crate::error::{BackendError, BackendResult};

/// The async backend facade over the data/daemon surface.
pub struct Backend {
    state: AppState,
    #[allow(dead_code)] // wired into dispatch methods in a later phase.
    runner: CommandRunner,
}

impl Backend {
    /// Build a backend from a layered [`TdwConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Init`] if the daemon composition root cannot be
    /// constructed from `config`.
    pub async fn from_config(config: TdwConfig) -> BackendResult<Self> {
        let state = AppState::from_config(config)
            .await
            .map_err(|error| BackendError::Init(error.to_string()))?;
        Ok(Self {
            state,
            runner: CommandRunner::default(),
        })
    }

    /// Build a backend backed by deterministic in-memory engines, for tests.
    pub async fn in_memory_for_tests() -> Self {
        Self {
            state: AppState::in_memory_for_tests().await,
            runner: CommandRunner::default(),
        }
    }

    /// The underlying daemon composition root.
    #[must_use]
    pub fn app_state(&self) -> &AppState {
        &self.state
    }
}
