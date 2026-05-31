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
