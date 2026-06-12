//! Phase 0 smoke test: both facades build from defaults.

use tdw_backend::agent::AgentBackend;
use tdw_backend::config::BackendConfig;
use tdw_backend::data::Backend;

#[tokio::test]
async fn builds_both_facades() {
    // Data facade: deterministic in-memory engines.
    let data = Backend::in_memory_for_tests().await;
    // The composition root is reachable.
    assert_eq!(data.app_state().config.profile, "default");

    // Agent facade: default config resolves an empty registry (no
    // TDW_AGENT_REGISTRY_DIR set) and an env-derived command policy.
    let config = BackendConfig::default();
    let agent = AgentBackend::from_config(&config);
    assert!(agent.is_ok(), "agent facade should construct from defaults");
}

/// Gate (K-X8): `Backend::in_memory_for_tests()` exposes a non-poisoned
/// `question_store_handle()`, proving that the composition root wires the
/// open-question store end-to-end (from_config production-registration gate).
#[tokio::test]
async fn in_memory_backend_wires_question_store() {
    let backend = Backend::in_memory_for_tests().await;
    let store = backend.question_store_handle();
    let guard = store.lock().expect("question store must not be poisoned");
    // A fresh backend has zero questions.
    assert_eq!(guard.total_count(), 0);
    assert_eq!(guard.open_count(), 0);
}
