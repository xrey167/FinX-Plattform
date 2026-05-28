#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used)]

// Drop-in integration tests for tdw-config — error variants, layer precedence,
// merge edge cases, and TOML parse failures.
//
// Drop at: crates/tdw-config/tests/layers.rs
// IMPORTANT: Authored offline. Verify with
// `cargo test --package tdw-config` before merging.

use serde_json::json;
use tdw_config::{
    ConfigError, ConfigLayer, ConfigLayerKind, DaemonTransport, PermissionAction, TdwConfig,
    WorkerBackend, config_schema, default_layer_order, merge_layers, schema_bundle,
};

#[test]
fn config_error_toml_displays_layer_name() {
    let err = ConfigLayer::from_toml(ConfigLayerKind::ProjectConfig, "broken", "= not toml")
        .expect_err("malformed TOML must error");
    let rendered = err.to_string();
    assert!(matches!(err, ConfigError::Toml { .. }));
    assert!(
        rendered.contains("broken"),
        "ConfigError::Toml display should include layer name, got: {rendered}"
    );
}

#[test]
fn default_layer_order_has_six_layers_in_ascending_precedence() {
    let order = default_layer_order();
    assert_eq!(order.len(), 6);
    let kinds: Vec<_> = order.iter().map(|layer| layer.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ConfigLayerKind::UserDefaults,
            ConfigLayerKind::EnvFile,
            ConfigLayerKind::ProjectConfig,
            ConfigLayerKind::SplitFile,
            ConfigLayerKind::InlineEnv,
            ConfigLayerKind::CliFlags,
        ]
    );
    assert!(
        order
            .windows(2)
            .all(|pair| pair[0].precedence < pair[1].precedence)
    );
}

#[test]
fn cli_flags_outrank_inline_env_on_conflict() {
    let layers = vec![
        ConfigLayer::new(
            ConfigLayerKind::InlineEnv,
            "env",
            json!({ "model": { "model": "from-env" } }),
        ),
        ConfigLayer::new(
            ConfigLayerKind::CliFlags,
            "cli",
            json!({ "model": { "model": "from-cli" } }),
        ),
    ];
    let merged = merge_layers(&layers).unwrap_or_else(|e| panic!("merge: {e}"));
    assert_eq!(merged.model.model, "from-cli");
}

#[test]
fn lower_precedence_layer_passed_last_does_not_override() {
    // ProjectConfig has precedence 30; CliFlags has 60. Even if we pass
    // ProjectConfig second, sort_by_key in merge_layers should reorder it.
    let layers = vec![
        ConfigLayer::new(
            ConfigLayerKind::CliFlags,
            "cli",
            json!({ "model": { "model": "from-cli" } }),
        ),
        ConfigLayer::new(
            ConfigLayerKind::ProjectConfig,
            "project",
            json!({ "model": { "model": "from-project" } }),
        ),
    ];
    let merged = merge_layers(&layers).unwrap_or_else(|e| panic!("merge: {e}"));
    assert_eq!(
        merged.model.model, "from-cli",
        "merge_layers must sort by precedence before applying"
    );
}

#[test]
fn nested_keys_merge_without_clobbering_siblings() {
    let layers = vec![
        ConfigLayer::new(
            ConfigLayerKind::ProjectConfig,
            "project",
            json!({
                "daemon": {
                    "transport": "Uds",
                    "uds_path": "/tmp/tdw.sock",
                    "tcp_bind": "127.0.0.1:9000"
                }
            }),
        ),
        ConfigLayer::new(
            ConfigLayerKind::CliFlags,
            "cli",
            json!({
                "daemon": { "http_bind": "127.0.0.1:7777" }
            }),
        ),
    ];
    let merged = merge_layers(&layers).unwrap_or_else(|e| panic!("merge: {e}"));
    assert_eq!(merged.daemon.uds_path, "/tmp/tdw.sock");
    assert_eq!(merged.daemon.tcp_bind.as_deref(), Some("127.0.0.1:9000"));
    assert_eq!(merged.daemon.http_bind.as_deref(), Some("127.0.0.1:7777"));
    assert_eq!(merged.daemon.transport, DaemonTransport::Uds);
}

#[test]
fn default_config_has_stable_invariants() {
    let cfg = TdwConfig::default();
    assert_eq!(cfg.profile, "default");
    assert_eq!(cfg.daemon.transport, DaemonTransport::Tcp);
    assert_eq!(cfg.daemon.tcp_bind.as_deref(), Some("127.0.0.1:8787"));
    assert_eq!(cfg.permissions.default_action, PermissionAction::Ask);
    assert!(cfg.permissions.last_match_wins);
    assert_eq!(cfg.protocol.max_event_bytes, 1_048_576);
    assert!(cfg.protocol.replay_enabled);
    assert!(cfg.session.jsonl_archive);
    assert_eq!(cfg.worker.backend, WorkerBackend::Sqlite);
    assert_eq!(cfg.worker.sqlite_path, "~/.tdw/worker.sqlite");
    assert_eq!(cfg.worker.postgres_url_env, "TDW_POSTGRES_URL");
}

#[test]
fn from_toml_parses_partial_layer_keeping_defaults() {
    let layer = ConfigLayer::from_toml(
        ConfigLayerKind::ProjectConfig,
        "project",
        r#"
profile = "dev"

[protocol]
max_event_bytes = 2048
"#,
    )
    .unwrap_or_else(|e| panic!("toml parses: {e}"));

    let merged = merge_layers(&[layer]).unwrap_or_else(|e| panic!("merge: {e}"));
    assert_eq!(merged.profile, "dev");
    assert_eq!(merged.protocol.max_event_bytes, 2048);
    // unchanged defaults
    assert!(merged.protocol.replay_enabled);
    assert_eq!(merged.session.sqlite_path, "~/.tdw/session.sqlite");
    assert_eq!(merged.worker.backend, WorkerBackend::Sqlite);
}

#[test]
fn merge_with_zero_layers_returns_defaults() {
    let merged = merge_layers(&[]).unwrap_or_else(|e| panic!("merge: {e}"));
    let default = TdwConfig::default();
    assert_eq!(merged.profile, default.profile);
    assert_eq!(
        merged.protocol.max_event_bytes,
        default.protocol.max_event_bytes
    );
}

#[test]
fn permission_action_round_trips() {
    for action in [
        PermissionAction::Allow,
        PermissionAction::Ask,
        PermissionAction::Deny,
    ] {
        let encoded = serde_json::to_value(action).unwrap_or_else(|e| panic!("serialize: {e}"));
        let decoded: PermissionAction =
            serde_json::from_value(encoded).unwrap_or_else(|e| panic!("deserialize: {e}"));
        assert_eq!(decoded, action);
    }
}

#[test]
fn config_layer_kind_precedence_is_monotonic() {
    let kinds = [
        ConfigLayerKind::UserDefaults,
        ConfigLayerKind::EnvFile,
        ConfigLayerKind::ProjectConfig,
        ConfigLayerKind::SplitFile,
        ConfigLayerKind::InlineEnv,
        ConfigLayerKind::CliFlags,
    ];
    let precedences: Vec<u8> = kinds.iter().map(|k| k.precedence()).collect();
    assert!(precedences.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn schema_bundle_exports_named_schemas() {
    let bundle = schema_bundle();
    assert!(bundle.contains_key("tdw_config"));
    assert!(bundle.contains_key("config_layer_descriptor"));
    let raw = config_schema();
    assert!(raw.is_object());
}
