//! Offline, no-network example for the `tdw-service` daemon crate.
//!
//! Exercises the daemon's *config-resolution* layer without binding a socket or
//! serving: it shows `resolve_profile` precedence and builds the `AppState`
//! composition root from an in-memory `TdwConfig`, then reports which transport
//! the daemon would bind and whether a policy is attached. No daemon is started,
//! no network, no Docker.
//!
//! Run with: `cargo run -p tdw-service --example tdw_service_basic`

use tdw_backend::server::resolve_profile;
use tdw_config::{DaemonTransport, TdwConfig};
use tdw_service_api::AppState;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Profile resolution: a non-empty TDW_PROFILE overrides the config value;
    //    a blank/absent one keeps the current profile. (Pure — no env mutation.)
    println!(
        "resolve_profile(default, Some(docker)) = {}",
        resolve_profile("default", Some("docker".to_string()))
    );
    println!(
        "resolve_profile(service, None)         = {}",
        resolve_profile("service", None)
    );

    // 2. Build an offline composition root from an in-memory config. This is the
    //    same AppState the daemon serves, minus the socket/serve lifecycle.
    let mut config = TdwConfig::default();
    config.session.sqlite_path = "sqlite::memory:".to_string();
    config.paths.rollout_dir = std::env::temp_dir()
        .join("tdw-service-example.jsonl")
        .to_string_lossy()
        .into_owned();
    config.daemon.transport = DaemonTransport::Tcp;
    config.daemon.tcp_bind = Some("127.0.0.1:7878".to_string());

    let state = AppState::from_config(config.clone()).await?;
    println!(
        "daemon would bind {:?} on {} (profile {})",
        config.daemon.transport,
        config.daemon.tcp_bind.as_deref().unwrap_or("<unset>"),
        config.profile,
    );
    println!(
        "policy attached: {} | registered providers: {}",
        state.policy.is_some(),
        state.registry.entries().len(),
    );

    Ok(())
}
