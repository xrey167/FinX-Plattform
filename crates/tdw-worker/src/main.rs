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
