//! Offline, no-network example for `tdw-config`.
//!
//! Demonstrates the layered merge: a project TOML layer is overridden by a
//! higher-precedence CLI-flag layer, and untouched values fall back to
//! `TdwConfig::default()`. No file or environment I/O — layer contents are
//! supplied inline, exactly as a caller would after reading them.
//!
//! Run with: `cargo run -p tdw-config --example tdw_config_basic`

use serde_json::json;
use tdw_config::{ConfigLayer, ConfigLayerKind, default_layer_order, merge_layers};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. A project-config layer parsed from TOML (precedence 30).
    let project = ConfigLayer::from_toml(
        ConfigLayerKind::ProjectConfig,
        "project",
        r#"
profile = "service"

[daemon]
transport = "Tcp"
tcp_bind = "127.0.0.1:7878"

[protocol]
max_event_bytes = 4096
"#,
    )?;

    // 2. A CLI-flag layer (precedence 60) overriding one leaf value.
    let cli = ConfigLayer::new(
        ConfigLayerKind::CliFlags,
        "cli",
        json!({ "protocol": { "max_event_bytes": 512 } }),
    );

    // 3. Merge: higher precedence wins per-leaf, siblings are preserved.
    let config = merge_layers(&[project, cli])?;
    assert_eq!(config.profile, "service");
    assert_eq!(config.daemon.tcp_bind.as_deref(), Some("127.0.0.1:7878"));
    assert_eq!(config.protocol.max_event_bytes, 512); // CLI override won.
    assert!(config.protocol.replay_enabled); // untouched default.

    println!(
        "merged profile={} tcp_bind={:?} max_event_bytes={} replay_enabled={}",
        config.profile,
        config.daemon.tcp_bind,
        config.protocol.max_event_bytes,
        config.protocol.replay_enabled,
    );

    // 4. The documented precedence order, lowest to highest.
    let order: Vec<&str> = default_layer_order().iter().map(|d| d.source).collect();
    println!("layer order (low -> high precedence): {order:?}");

    Ok(())
}
