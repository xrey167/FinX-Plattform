#![forbid(unsafe_code)]

use std::time::Duration;

use tdw_app_client::{DEFAULT_DAEMON_TCP_ADDR, DaemonClientConfig};
use tdw_app_server::{DaemonEndpoint, DaemonTransport};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("tdw-worker error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    if std::env::args().any(|arg| arg == "--contract") {
        println!("{}", tdw_worker::worker_contract_json());
        return Ok(());
    }

    if std::env::args().any(|arg| arg == "--durable-smoke") {
        println!(
            "{}",
            durable_smoke().await.map_err(|error| error.to_string())?
        );
        return Ok(());
    }

    if std::env::args().any(|arg| arg == "--serve" || arg == "--serve-once") {
        return serve().await;
    }

    match tdw_service_api::fetch_equity_historical("yahoo", "MSFT") {
        Ok(object) => match tdw_service_api::event_spine_sample("worker") {
            Ok(event) => {
                println!(
                    "tdw-worker job=equity_historical provider={} rows={} event_spine={}",
                    object.provider,
                    object.rows.len(),
                    event
                );
                Ok(())
            }
            Err(error) => Err(format!("tdw-worker event error: {error}")),
        },
        Err(error) => Err(error.to_string()),
    }
}

async fn serve() -> std::result::Result<(), String> {
    let once = std::env::args().any(|arg| arg == "--serve-once");
    let db_url =
        std::env::var("TDW_WORKER_DB").unwrap_or_else(|_| "sqlite://tdw-worker.sqlite".to_string());
    let queue = tdw_worker::SqliteWorkerQueue::connect(&db_url)
        .await
        .map_err(|error| error.to_string())?;
    let config = serve_config_from_env();
    let mode = if once {
        "serve-once draining"
    } else {
        "serving"
    };

    // Daemon dispatch when configured; otherwise the offline ack handler.
    let report = match daemon_dispatch_config()? {
        Some((daemon_config, endpoint)) => {
            eprintln!(
                "tdw-worker {mode} {db_url} (worker_id={}, daemon dispatch -> {endpoint})",
                config.worker_id
            );
            run_serve(queue, tdw_worker::DaemonJobHandler::new(daemon_config), config, once).await
        }
        None => {
            eprintln!(
                "tdw-worker {mode} {db_url} (worker_id={}, ack handler - set TDW_WORKER_DAEMON_ADDR for daemon dispatch)",
                config.worker_id
            );
            run_serve(queue, tdw_worker::LoggingAckHandler, config, once).await
        }
    }
    .map_err(|error| error.to_string())?;

    println!(
        "{}",
        serde_json::to_string(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

async fn run_serve<H: tdw_worker::JobHandler>(
    queue: tdw_worker::SqliteWorkerQueue,
    handler: H,
    config: tdw_worker::ServeConfig,
    once: bool,
) -> tdw_worker::Result<tdw_worker::ServeReport> {
    let runner = tdw_worker::WorkerRunner::new(queue, handler, config);
    if once {
        runner.run_until_idle().await
    } else {
        runner
            .run(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    }
}

/// Resolve daemon-dispatch config from the environment. Returns `None` (ack
/// handler) unless `TDW_WORKER_DISPATCH=daemon`, `TDW_WORKER_DAEMON_ADDR`, or
/// `TDW_WORKER_DAEMON_TRANSPORT` is set. Validates the endpoint, failing closed
/// on unsupported transports (e.g. UDS on Windows).
fn daemon_dispatch_config() -> std::result::Result<Option<(DaemonClientConfig, String)>, String> {
    let dispatch = non_empty_env("TDW_WORKER_DISPATCH");
    let addr = non_empty_env("TDW_WORKER_DAEMON_ADDR");
    let transport_raw = non_empty_env("TDW_WORKER_DAEMON_TRANSPORT");
    let enabled =
        matches!(dispatch.as_deref(), Some("daemon")) || addr.is_some() || transport_raw.is_some();
    if !enabled {
        return Ok(None);
    }

    let transport = match transport_raw.as_deref() {
        Some(value) => parse_daemon_transport(value)?,
        None => DaemonTransport::Tcp,
    };
    let address = addr.unwrap_or_else(|| default_daemon_address(transport));
    let timeout = match non_empty_env("TDW_WORKER_DAEMON_TIMEOUT_MS") {
        Some(value) => Duration::from_millis(
            value
                .parse()
                .map_err(|error| format!("invalid TDW_WORKER_DAEMON_TIMEOUT_MS: {error}"))?,
        ),
        None => Duration::from_secs(2),
    };

    let endpoint_label = format!("{}:{address}", daemon_transport_label(transport));
    let daemon_config =
        DaemonClientConfig::new(DaemonEndpoint { transport, address }).with_timeout(timeout);
    daemon_config
        .validate()
        .map_err(|error| format!("invalid daemon client config: {error:?}"))?;
    Ok(Some((daemon_config, endpoint_label)))
}

fn parse_daemon_transport(value: &str) -> std::result::Result<DaemonTransport, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "tcp" => Ok(DaemonTransport::Tcp),
        "uds" | "unix" => Ok(DaemonTransport::Uds),
        "http-sse" | "http" | "sse" => Ok(DaemonTransport::HttpSse),
        other => Err(format!("unsupported TDW_WORKER_DAEMON_TRANSPORT: {other}")),
    }
}

fn default_daemon_address(transport: DaemonTransport) -> String {
    match transport {
        DaemonTransport::Tcp => DEFAULT_DAEMON_TCP_ADDR.to_string(),
        DaemonTransport::Uds => "/tmp/tdw-daemon.sock".to_string(),
        DaemonTransport::HttpSse => "http://127.0.0.1:7879/events".to_string(),
    }
}

const fn daemon_transport_label(transport: DaemonTransport) -> &'static str {
    match transport {
        DaemonTransport::Tcp => "tcp",
        DaemonTransport::Uds => "uds",
        DaemonTransport::HttpSse => "http-sse",
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn serve_config_from_env() -> tdw_worker::ServeConfig {
    let mut config = tdw_worker::ServeConfig::default();
    match std::env::var("TDW_WORKER_ID") {
        Ok(id) if !id.trim().is_empty() => config.worker_id = id,
        _ => {}
    }
    if let Some(ttl) = std::env::var("TDW_WORKER_LEASE_TTL_MS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        config.lease_ttl_ms = ttl;
    }
    if let Some(poll) = std::env::var("TDW_WORKER_POLL_MS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        config.poll_interval_ms = poll;
    }
    config
}

async fn durable_smoke() -> tdw_worker::Result<String> {
    let queue = tdw_worker::SqliteWorkerQueue::connect("sqlite::memory:").await?;
    let outcome = queue
        .enqueue(tdw_worker::sample_shutdown_job("worker-smoke-1")?)
        .await?;
    let lease = queue.lease_next("worker-smoke").await?;
    if let Some(lease) = &lease {
        queue.complete(&lease.job_id).await?;
    }
    let stats = queue.stats().await?;

    Ok(serde_json::json!({
        "worker": "tdw-worker",
        "durable_smoke": true,
        "backend": "sqlite",
        "inserted": outcome.inserted,
        "lease": lease,
        "stats": stats
    })
    .to_string())
}
