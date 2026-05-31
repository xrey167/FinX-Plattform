#![forbid(unsafe_code)]

use tdw_backend::server::{load_config, run_daemon};
use tdw_test_utils::smoke::{allocate_storage_root, run_end_to_end_smoke};

pub type ServiceError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), ServiceError> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--smoke") {
        let symbol = args
            .iter()
            .position(|a| a == "--smoke")
            .and_then(|i| args.get(i + 1))
            .map_or("AAPL", |s| s.as_str());
        let root = allocate_storage_root("tdw-service-smoke");
        let report = run_end_to_end_smoke(symbol, root.clone())
            .await
            .map_err(|e| format!("smoke error: {e}"))?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| format!("serialize error: {e}"))?
        );
        let _ = std::fs::remove_dir_all(&root);
        return Ok(());
    }

    // Daemon mode. The bootstrap (config resolution, AppState wiring, relay +
    // transport spawn, serve-until-ctrl-c) is factored into `tdw-backend`'s
    // `server` module so the standalone binary and the unified `tdw-backend`
    // binary share one implementation. This crate stays a thin entrypoint.
    let config = load_config().await?;
    run_daemon(&config).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tdw_app_server::{CancellationToken, service_channel};
    use tdw_backend::server::{resolve_profile, spawn_transport};
    use tdw_config::{DaemonTransport, TdwConfig};
    use tdw_service_api::AppState;

    #[test]
    fn resolve_profile_prefers_non_empty_env_override() {
        // A non-empty TDW_PROFILE overrides the config-derived profile, so the
        // compose `live` stack's `TDW_PROFILE: docker` is actually applied.
        assert_eq!(
            resolve_profile("default", Some("docker".to_string())),
            "docker"
        );
        // Surrounding whitespace is trimmed.
        assert_eq!(
            resolve_profile("default", Some("  production  ".to_string())),
            "production"
        );
    }

    #[test]
    fn resolve_profile_keeps_current_when_env_absent_or_blank() {
        assert_eq!(resolve_profile("service", None), "service");
        assert_eq!(resolve_profile("service", Some(String::new())), "service");
        assert_eq!(
            resolve_profile("service", Some("   ".to_string())),
            "service"
        );
    }

    #[cfg(not(feature = "transport-http"))]
    #[tokio::test]
    async fn http_transport_without_feature_fails_startup() {
        let state = AppState::in_memory_for_tests().await;
        let (handle, events_rx, _service_loop) = service_channel(state.clone(), state);
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::HttpSse;

        let result = spawn_transport(&config, handle, events_rx, CancellationToken::new()).await;
        let Err(error) = result else {
            panic!("HTTP/SSE should fail when transport-http is not compiled")
        };

        assert!(error.to_string().contains("transport-http"));
    }

    #[cfg(not(all(unix, feature = "transport-uds")))]
    #[tokio::test]
    async fn uds_transport_without_feature_fails_startup() {
        let state = AppState::in_memory_for_tests().await;
        let (handle, events_rx, _service_loop) = service_channel(state.clone(), state);
        let mut config = TdwConfig::default();
        config.daemon.transport = DaemonTransport::Uds;

        let result = spawn_transport(&config, handle, events_rx, CancellationToken::new()).await;
        let Err(error) = result else {
            panic!("UDS should fail when transport-uds is not compiled for Unix")
        };

        assert!(error.to_string().contains("transport-uds"));
    }
}
