//! Integration test for the Postgres-backed distributed worker queue.
//!
//! Runs only with `--features postgres`; if `TDW_POSTGRES_TEST_URL` is not set,
//! the test compiles and exits without touching a database.

#![cfg(feature = "postgres")]

use tdw_worker::{PgWorkerQueue, WorkerJobStatus, sample_shutdown_job};

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn test_prefix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("tdw_worker_pg_test_{}_{}_", std::process::id(), nanos)
}

async fn cleanup(queue: &PgWorkerQueue, prefix: &str) {
    let job_ids = [
        format!("{prefix}low"),
        format!("{prefix}high"),
        format!("{prefix}dead"),
    ];
    for job_id in job_ids {
        if queue.job_status(&job_id).await.unwrap_or(None).is_some() {
            let _ = queue.complete(&job_id).await;
        }
    }
}

#[tokio::test]
async fn pg_worker_queue_full_surface_roundtrip() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping pg worker integration test");
        return;
    };

    let queue = PgWorkerQueue::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres worker connect: {error}"));
    let prefix = test_prefix();
    cleanup(&queue, &prefix).await;

    let mut low = sample_shutdown_job(&format!("{prefix}low")).expect("low job");
    low.priority = 1;
    let mut high = sample_shutdown_job(&format!("{prefix}high")).expect("high job");
    high.priority = 10;

    assert!(
        queue
            .enqueue(low.clone())
            .await
            .unwrap_or_else(|error| panic!("enqueue low: {error}"))
            .inserted
    );
    assert!(
        !queue
            .enqueue(low)
            .await
            .unwrap_or_else(|error| panic!("idempotent enqueue low: {error}"))
            .inserted
    );
    queue
        .enqueue(high)
        .await
        .unwrap_or_else(|error| panic!("enqueue high: {error}"));

    let leased = queue
        .lease_next("pg-worker-1")
        .await
        .unwrap_or_else(|error| panic!("lease next: {error}"))
        .expect("priority lease");
    assert_eq!(leased.job_id, format!("{prefix}high"));
    assert_eq!(leased.attempt, 1);
    queue
        .complete(&leased.job_id)
        .await
        .unwrap_or_else(|error| panic!("complete high: {error}"));
    queue
        .complete(&leased.job_id)
        .await
        .unwrap_or_else(|error| panic!("idempotent complete high: {error}"));
    assert_eq!(
        queue
            .job_status(&leased.job_id)
            .await
            .unwrap_or_else(|error| panic!("status high: {error}")),
        Some(WorkerJobStatus::Completed)
    );

    let mut dead = sample_shutdown_job(&format!("{prefix}dead")).expect("dead job");
    dead.max_attempts = 1;
    dead.priority = 20;
    queue
        .enqueue(dead)
        .await
        .unwrap_or_else(|error| panic!("enqueue dead: {error}"));
    let dead_lease = queue
        .lease_next_with_ttl("pg-worker-2", 0)
        .await
        .unwrap_or_else(|error| panic!("lease dead: {error}"))
        .expect("dead lease");
    assert_eq!(dead_lease.job_id, format!("{prefix}dead"));
    assert_eq!(dead_lease.attempt, 1);
    assert_eq!(
        queue
            .reap_expired_leases()
            .await
            .unwrap_or_else(|error| panic!("reap expired: {error}")),
        1
    );
    let letters = queue
        .dead_letters()
        .await
        .unwrap_or_else(|error| panic!("dead letters: {error}"));
    assert!(
        letters
            .iter()
            .any(|letter| letter.job.job_id == dead_lease.job_id
                && letter.last_error.as_deref() == Some("lease expired"))
    );

    // Replay re-enqueues the dead-lettered job with a fresh attempt counter.
    queue
        .replay_dead_letter(&dead_lease.job_id)
        .await
        .unwrap_or_else(|error| panic!("replay dead letter: {error}"));
    assert_eq!(
        queue
            .job_status(&dead_lease.job_id)
            .await
            .unwrap_or_else(|error| panic!("status after replay: {error}")),
        Some(WorkerJobStatus::Pending)
    );
    let replayed = queue
        .lease_next("pg-worker-3")
        .await
        .unwrap_or_else(|error| panic!("lease replayed: {error}"))
        .expect("replayed lease");
    assert_eq!(replayed.job_id, dead_lease.job_id);
    assert_eq!(replayed.attempt, 1, "replay resets the attempt counter");
    assert!(matches!(
        queue.replay_dead_letter("missing-job-id").await,
        Err(tdw_worker::WorkerQueueError::UnknownJob(_))
    ));

    let stats = queue
        .stats()
        .await
        .unwrap_or_else(|error| panic!("stats: {error}"));
    assert!(stats.completed >= 1);

    cleanup(&queue, &prefix).await;
}
