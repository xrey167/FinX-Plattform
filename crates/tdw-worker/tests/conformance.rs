//! Cross-backend behavioral conformance for the worker queue (go-live P2.6).
//!
//! ONE suite of assertions runs verbatim against every backend so the
//! backends cannot silently drift apart: a workflow prototyped on the
//! in-memory queue must behave identically when promoted to `SQLite` or
//! Postgres. This pins the contract the audit items WK1/WK3 fixed
//! (duplicate-`job_id` rejection, idempotent complete) plus priority
//! ordering and the retry → dead-letter path.
//!
//! - in-memory and `sqlite::memory:` legs always run;
//! - the Postgres leg compiles with `--features postgres` and self-skips
//!   unless `TDW_POSTGRES_TEST_URL` is set (same convention as
//!   `tests/pg_worker.rs`).

use tdw_worker::{
    DurableWorkerQueue, EnqueueOutcome, InMemoryWorkerQueue, SqliteWorkerQueue, WorkerJobStatus,
    sample_shutdown_job,
};

#[cfg(feature = "postgres")]
use tdw_worker::PgWorkerQueue;

const LEASE_TTL_MS: u64 = 60_000;

/// Uniform async facade over the three backends. The in-memory queue is
/// synchronous; SQLite/Postgres are async — the conformance suite must not
/// care, so each arm normalizes to the same `async` signatures.
enum AnyQueue {
    Mem(InMemoryWorkerQueue),
    Sqlite(SqliteWorkerQueue),
    #[cfg(feature = "postgres")]
    Pg(PgWorkerQueue),
}

impl AnyQueue {
    async fn enqueue(&mut self, job: tdw_worker::WorkerJob) -> EnqueueOutcome {
        match self {
            Self::Mem(q) => q.enqueue(job),
            Self::Sqlite(q) => q.enqueue(job).await,
            #[cfg(feature = "postgres")]
            Self::Pg(q) => q.enqueue(job).await,
        }
        .unwrap_or_else(|error| panic!("enqueue must succeed: {error}"))
    }

    async fn lease_next(&mut self, worker_id: &str) -> Option<tdw_worker::WorkerLease> {
        match self {
            Self::Mem(q) => q.lease_next_with_ttl(worker_id, LEASE_TTL_MS),
            Self::Sqlite(q) => q.lease_next_with_ttl(worker_id, LEASE_TTL_MS).await,
            #[cfg(feature = "postgres")]
            Self::Pg(q) => q.lease_next_with_ttl(worker_id, LEASE_TTL_MS).await,
        }
        .unwrap_or_else(|error| panic!("lease_next must succeed: {error}"))
    }

    async fn complete(&mut self, job_id: &str) -> tdw_worker::Result<()> {
        match self {
            Self::Mem(q) => q.complete(job_id),
            Self::Sqlite(q) => q.complete(job_id).await,
            #[cfg(feature = "postgres")]
            Self::Pg(q) => q.complete(job_id).await,
        }
    }

    async fn fail(&mut self, job_id: &str, error: &str, retry_after_ms: u64) -> WorkerJobStatus {
        match self {
            Self::Mem(q) => q.fail(job_id, error, retry_after_ms),
            Self::Sqlite(q) => q.fail(job_id, error, retry_after_ms).await,
            #[cfg(feature = "postgres")]
            Self::Pg(q) => q.fail(job_id, error, retry_after_ms).await,
        }
        .unwrap_or_else(|error| panic!("fail must succeed: {error}"))
    }
}

/// The behavioral contract, asserted identically for every backend.
async fn conformance_suite(queue: &mut AnyQueue, prefix: &str) {
    let mid_id = format!("{prefix}mid");
    let low_id = format!("{prefix}low");
    let high_id = format!("{prefix}high");

    // 1. Idempotent enqueue: a duplicate job_id is acknowledged, not inserted
    //    and not an error.
    let mut mid = sample_shutdown_job(&mid_id).expect("mid job builds");
    mid.priority = 5;
    mid.max_attempts = 2;
    let first = queue.enqueue(mid.clone()).await;
    assert!(first.inserted, "first enqueue inserts");
    assert_eq!(first.job_id, mid_id);
    let dup = queue.enqueue(mid).await;
    assert!(!dup.inserted, "duplicate job_id must not insert");
    assert_eq!(dup.job_id, mid_id);

    // 2. Priority ordering: higher priority leases first regardless of
    //    insertion order.
    let mut low = sample_shutdown_job(&low_id).expect("low job builds");
    low.priority = 1;
    let mut high = sample_shutdown_job(&high_id).expect("high job builds");
    high.priority = 10;
    assert!(queue.enqueue(low).await.inserted);
    assert!(queue.enqueue(high).await.inserted);

    let lease1 = queue.lease_next("w1").await.expect("a job is leasable");
    assert_eq!(lease1.job_id, high_id, "highest priority leases first");
    assert_eq!(lease1.attempt, 1);
    let lease2 = queue.lease_next("w1").await.expect("next job is leasable");
    assert_eq!(lease2.job_id, mid_id, "middle priority leases second");
    let lease3 = queue.lease_next("w1").await.expect("last job is leasable");
    assert_eq!(lease3.job_id, low_id, "lowest priority leases last");
    assert!(
        queue.lease_next("w1").await.is_none(),
        "no further pending jobs"
    );

    // 3. Idempotent complete: a second complete of the same job is a no-op,
    //    not an error.
    queue
        .complete(&high_id)
        .await
        .unwrap_or_else(|error| panic!("first complete: {error}"));
    queue
        .complete(&high_id)
        .await
        .unwrap_or_else(|error| panic!("duplicate complete must be a no-op: {error}"));

    // 4. Retry then dead-letter: with max_attempts = 2 the first failure
    //    re-pends the job, the second exhausts it.
    let after_first_failure = queue.fail(&mid_id, "boom", 0).await;
    assert_eq!(
        after_first_failure,
        WorkerJobStatus::Pending,
        "attempt 1 of 2 failing must re-pend"
    );
    let retry = queue.lease_next("w2").await.expect("retry is leasable");
    assert_eq!(retry.job_id, mid_id);
    assert_eq!(retry.attempt, 2, "retry increments the attempt");
    let after_second_failure = queue.fail(&mid_id, "boom again", 0).await;
    assert_eq!(
        after_second_failure,
        WorkerJobStatus::DeadLettered,
        "exhausting max_attempts dead-letters"
    );
    // `low` is still leased by w1 and `mid` is dead-lettered: nothing may lease.
    assert!(
        queue.lease_next("w2").await.is_none(),
        "dead-lettered job must never lease again"
    );

    // 5. The remaining leased job completes normally.
    queue
        .complete(&low_id)
        .await
        .unwrap_or_else(|error| panic!("complete low: {error}"));
}

#[tokio::test]
async fn in_memory_backend_conforms() {
    let mut queue = AnyQueue::Mem(InMemoryWorkerQueue::default());
    conformance_suite(&mut queue, "conf_mem_").await;
}

#[tokio::test]
async fn sqlite_backend_conforms() {
    let sqlite = SqliteWorkerQueue::connect("sqlite::memory:")
        .await
        .unwrap_or_else(|error| panic!("sqlite connect: {error}"));
    let mut queue = AnyQueue::Sqlite(sqlite);
    conformance_suite(&mut queue, "conf_sqlite_").await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_conforms() {
    let Some(url) = std::env::var("TDW_POSTGRES_TEST_URL").ok() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping postgres conformance leg");
        return;
    };
    let pg = PgWorkerQueue::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres connect: {error}"));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let prefix = format!("conf_pg_{}_{}_", std::process::id(), nanos);
    let mut queue = AnyQueue::Pg(pg);
    conformance_suite(&mut queue, &prefix).await;
}
