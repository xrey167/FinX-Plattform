#![forbid(unsafe_code)]

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
        return serve().await.map_err(|error| error.to_string());
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

async fn serve() -> tdw_worker::Result<()> {
    let once = std::env::args().any(|arg| arg == "--serve-once");
    let db_url =
        std::env::var("TDW_WORKER_DB").unwrap_or_else(|_| "sqlite://tdw-worker.sqlite".to_string());
    let queue = tdw_worker::SqliteWorkerQueue::connect(&db_url).await?;

    let config = serve_config_from_env();
    if once {
        eprintln!("tdw-worker serve-once draining {db_url}");
    } else {
        eprintln!(
            "tdw-worker serving {db_url} (worker_id={}, lease_ttl_ms={}); Ctrl-C to drain and stop",
            config.worker_id, config.lease_ttl_ms
        );
    }

    let runner = tdw_worker::WorkerRunner::new(queue, tdw_worker::LoggingAckHandler, config);
    let report = if once {
        runner.run_until_idle().await?
    } else {
        runner
            .run(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?
    };

    println!("{}", serde_json::to_string(&report)?);
    Ok(())
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
