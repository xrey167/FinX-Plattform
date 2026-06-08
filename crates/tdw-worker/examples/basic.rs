//! Offline, no-network example for `tdw-worker`.
//!
//! Drives the in-memory durable queue (the contract reference backend) through
//! the lease lifecycle: enqueue → lease → complete, then prints queue stats. The
//! in-memory backend is fully synchronous, so no tokio runtime, no daemon, and
//! no SQLite/Postgres are involved.
//!
//! Run with: `cargo run -p tdw-worker --example tdw_worker_basic`

use tdw_worker::{DurableWorkerQueue, InMemoryWorkerQueue, WorkerJobStatus, sample_shutdown_job};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut queue = InMemoryWorkerQueue::default();

    // 1. Enqueue a job (its payload is an Op::Shutdown OpEnvelope). Idempotent on
    //    job_id: a second enqueue of the same id reports inserted = false.
    let outcome = queue.enqueue(sample_shutdown_job("job-1")?)?;
    println!("enqueue job-1: inserted={}", outcome.inserted);
    let dup = queue.enqueue(sample_shutdown_job("job-1")?);
    println!("re-enqueue job-1 errors as duplicate: {}", dup.is_err());

    // 2. Lease the next ready job for a worker, then complete it.
    let lease = queue.lease_next("worker-a")?.expect("a ready job to lease");
    println!(
        "leased {} by {} (attempt {})",
        lease.job_id, lease.worker_id, lease.attempt
    );
    queue.complete(&lease.job_id)?;

    // 3. Stats reflect the completed job.
    let stats = queue.stats();
    println!(
        "stats: pending={} leased={} completed={} dead_lettered={}",
        stats.pending, stats.leased, stats.completed, stats.dead_lettered
    );
    assert_eq!(stats.completed, 1);

    // 4. A job that exhausts its attempts dead-letters; show fail() once.
    queue.enqueue({
        let mut job = sample_shutdown_job("job-2")?;
        job.max_attempts = 1;
        job
    })?;
    let lease2 = queue.lease_next("worker-a")?.expect("job-2 ready");
    let status = queue.fail(&lease2.job_id, "boom", 0)?;
    println!("job-2 after 1 failed attempt: {status:?}");
    assert_eq!(status, WorkerJobStatus::DeadLettered);
    println!("dead letters: {}", queue.dead_letters().len());

    Ok(())
}
