#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::time::Duration;

use tdw_app_server::{
    CancellationToken, SubmissionHandle, serve, service_channel, spawn_inmemory_relay,
};
use tdw_config::{DaemonTransport, TdwConfig};
use tdw_protocol::EventMsg;
use tdw_service_api::AppState;
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

    // Daemon mode.
    let config = load_config().await?;
    let state = AppState::from_config(config.clone())
        .await
        .map_err(|e| format!("AppState::from_config failed: {e}"))?;

    // Note: no policy attached here by design — daemon will respond with
    // Failed for any dispatch until a policy is attached (P7 will wire this).
    eprintln!(
        "tdw-service: daemon starting (no policy attached; dispatches will return Failed until P7)"
    );

    let (handle, events_rx, service_loop) = service_channel(state.clone(), state.clone());

    let cancel = CancellationToken::new();

    let relay = spawn_inmemory_relay(
        state.outbox.clone(),
        state.bus.clone(),
        Duration::from_millis(50),
        cancel.clone(),
    );

    let transport_addr = effective_transport_addr(&config);
    let transport_task = spawn_transport(&config, handle, events_rx, cancel.clone()).await?;

    println!(
        "tdw-service: daemon up. transport={:?} addr={}. ctrl-c to exit.",
        config.daemon.transport, transport_addr
    );

    serve(service_loop, relay, cancel.clone()).await?;
    let _ = transport_task.await;

    Ok(())
}

async fn load_config() -> Result<TdwConfig, ServiceError> {
    // Honour TDW_CONFIG env var if set: read that TOML file and merge layers.
    if let Ok(path) = std::env::var("TDW_CONFIG") {
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("TDW_CONFIG read error ({path}): {e}"))?;
        let layer = tdw_config::ConfigLayer::from_toml(
            tdw_config::ConfigLayerKind::EnvFile,
            "TDW_CONFIG",
            &contents,
        )
        .map_err(|e| format!("TDW_CONFIG parse error: {e}"))?;
        let mut config =
            tdw_config::merge_layers(&[layer]).map_err(|e| format!("config merge error: {e}"))?;
        // Override volatile paths to safe in-memory defaults for daemon boot.
        config.session.sqlite_path = "sqlite::memory:".to_string();
        let rollout = std::env::temp_dir()
            .join("tdw-rollout.jsonl")
            .to_string_lossy()
            .into_owned();
        config.paths.rollout_dir = rollout;
        return Ok(config);
    }

    // Default minimal config: in-memory session store + tmp rollout.
    let mut config = TdwConfig::default();
    config.session.sqlite_path = "sqlite::memory:".to_string();
    config.paths.rollout_dir = std::env::temp_dir()
        .join("tdw-rollout.jsonl")
        .to_string_lossy()
        .into_owned();
    // Use a fixed local TCP port so the daemon is reachable on all platforms.
    config.daemon.transport = DaemonTransport::Tcp;
    config.daemon.tcp_bind = Some("127.0.0.1:7878".to_string());
    Ok(config)
}

/// Returns a human-readable description of the address we will bind.
fn effective_transport_addr(config: &TdwConfig) -> String {
    match config.daemon.transport {
        DaemonTransport::Tcp => config
            .daemon
            .tcp_bind
            .clone()
            .unwrap_or_else(|| "127.0.0.1:7878".to_string()),
        DaemonTransport::Uds => {
            // UDS not available on Windows; we fall back to TCP.
            #[cfg(not(unix))]
            {
                config
                    .daemon
                    .tcp_bind
                    .clone()
                    .unwrap_or_else(|| "127.0.0.1:7878".to_string())
            }
            #[cfg(unix)]
            {
                config.daemon.uds_path.clone()
            }
        }
        DaemonTransport::HttpSse => config
            .daemon
            .http_bind
            .clone()
            .unwrap_or_else(|| "127.0.0.1:7879".to_string()),
    }
}

async fn spawn_transport(
    config: &TdwConfig,
    handle: SubmissionHandle,
    events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    cancel: CancellationToken,
) -> Result<tokio::task::JoinHandle<std::io::Result<()>>, ServiceError> {
    match config.daemon.transport {
        DaemonTransport::Tcp => {
            let addr: SocketAddr = config
                .daemon
                .tcp_bind
                .as_deref()
                .unwrap_or("127.0.0.1:7878")
                .parse()
                .map_err(|e| format!("invalid tcp_bind address: {e}"))?;
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| format!("TCP bind failed ({addr}): {e}"))?;
            let bound = listener
                .local_addr()
                .map_err(|e| format!("local_addr: {e}"))?;
            eprintln!("tdw-service: TCP listener bound on {bound}");
            Ok(tokio::spawn(async move {
                tdw_app_server::serve_tcp(listener, handle, events_rx, cancel).await
            }))
        }

        DaemonTransport::Uds => spawn_uds(config, handle, events_rx, cancel).await,

        DaemonTransport::HttpSse => spawn_http(config, handle, events_rx, cancel).await,
    }
}

// Feature-gated UDS helper — only compiled on Unix with the transport-uds feature.
#[cfg(all(unix, feature = "transport-uds"))]
async fn spawn_uds(
    config: &TdwConfig,
    handle: SubmissionHandle,
    events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    cancel: CancellationToken,
) -> Result<tokio::task::JoinHandle<std::io::Result<()>>, ServiceError> {
    use std::path::PathBuf;
    let path = PathBuf::from(&config.daemon.uds_path);
    eprintln!("tdw-service: UDS listener on {:?}", path);
    Ok(tokio::spawn(async move {
        tdw_app_server::serve_uds(path, handle, events_rx, cancel).await
    }))
}

// UDS is explicit: fail startup instead of silently binding a different transport.
#[cfg(not(all(unix, feature = "transport-uds")))]
async fn spawn_uds(
    _config: &TdwConfig,
    _handle: SubmissionHandle,
    _events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    _cancel: CancellationToken,
) -> Result<tokio::task::JoinHandle<std::io::Result<()>>, ServiceError> {
    Err("UDS transport requested but this binary was not built for transport-uds on Unix".into())
}

// Feature-gated HTTP/SSE helper.
#[cfg(feature = "transport-http")]
async fn spawn_http(
    config: &TdwConfig,
    handle: SubmissionHandle,
    events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    cancel: CancellationToken,
) -> Result<tokio::task::JoinHandle<std::io::Result<()>>, ServiceError> {
    let bind_str = config
        .daemon
        .http_bind
        .as_deref()
        .unwrap_or("127.0.0.1:7879");
    let addr: SocketAddr = bind_str
        .parse()
        .map_err(|e| format!("invalid http_bind address: {e}"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("HTTP bind failed ({addr}): {e}"))?;
    let bound = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?;
    eprintln!("tdw-service: HTTP/SSE listener bound on {bound}");
    Ok(tokio::spawn(async move {
        tdw_app_server::serve_http(listener, handle, events_rx, cancel).await
    }))
}

// HTTP/SSE is explicit: fail startup instead of silently binding a different transport.
#[cfg(not(feature = "transport-http"))]
async fn spawn_http(
    _config: &TdwConfig,
    _handle: SubmissionHandle,
    _events_rx: tokio::sync::mpsc::UnboundedReceiver<EventMsg>,
    _cancel: CancellationToken,
) -> Result<tokio::task::JoinHandle<std::io::Result<()>>, ServiceError> {
    Err(
        "HTTP/SSE transport requested but this binary was not built with feature 'transport-http'"
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
