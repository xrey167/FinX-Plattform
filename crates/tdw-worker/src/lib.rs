#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tdw_protocol::OpEnvelope;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerJob {
    pub job_id: String,
    pub queue: String,
    pub envelope: OpEnvelope,
    pub max_attempts: u32,
    pub not_before_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerLease {
    pub job_id: String,
    pub worker_id: String,
    pub attempt: u32,
}

pub trait DurableWorkerQueue {
    fn enqueue(&mut self, job: WorkerJob) -> Result<(), String>;
    fn lease_next(&mut self, worker_id: &str) -> Result<Option<WorkerLease>, String>;
    fn complete(&mut self, job_id: &str) -> Result<(), String>;
}

#[derive(Default)]
pub struct InMemoryWorkerQueue {
    jobs: Vec<WorkerJob>,
    leased: Vec<WorkerLease>,
}

impl DurableWorkerQueue for InMemoryWorkerQueue {
    fn enqueue(&mut self, job: WorkerJob) -> Result<(), String> {
        validate_job(&job)?;
        if self
            .jobs
            .iter()
            .any(|existing| existing.job_id == job.job_id)
        {
            return Err(format!("duplicate job_id: {}", job.job_id));
        }
        self.jobs.push(job);
        Ok(())
    }

    fn lease_next(&mut self, worker_id: &str) -> Result<Option<WorkerLease>, String> {
        if worker_id.trim().is_empty() {
            return Err("worker_id must not be empty".to_string());
        }
        let now_ms = unix_epoch_millis()?;
        let Some(job) = self.jobs.iter().find(|job| {
            job.not_before_ms <= now_ms
                && !self.leased.iter().any(|lease| lease.job_id == job.job_id)
        }) else {
            return Ok(None);
        };
        let lease = WorkerLease {
            job_id: job.job_id.clone(),
            worker_id: worker_id.to_string(),
            attempt: 1,
        };
        self.leased.push(lease.clone());
        Ok(Some(lease))
    }

    fn complete(&mut self, job_id: &str) -> Result<(), String> {
        let Some(index) = self.jobs.iter().position(|job| job.job_id == job_id) else {
            return Err(format!("unknown job_id: {job_id}"));
        };
        self.jobs.remove(index);
        self.leased.retain(|lease| lease.job_id != job_id);
        Ok(())
    }
}

pub fn worker_contract_json() -> String {
    serde_json::json!({
        "contract": "tdw.worker.queue.v1",
        "job": {
            "job_id": "stable idempotency key",
            "queue": "logical queue name",
            "envelope": "tdw_protocol::OpEnvelope",
            "max_attempts": "positive integer",
            "not_before_ms": "unix epoch milliseconds"
        },
        "operations": ["enqueue", "lease_next", "complete"],
        "shipped_now": "typed boundary and in-memory contract tests",
        "later": "durable SQL/RiverQueue-backed scheduler implementation"
    })
    .to_string()
}

fn validate_job(job: &WorkerJob) -> Result<(), String> {
    if job.job_id.trim().is_empty() {
        return Err("job_id must not be empty".to_string());
    }
    if job.queue.trim().is_empty() {
        return Err("queue must not be empty".to_string());
    }
    if job.max_attempts == 0 {
        return Err("max_attempts must be greater than zero".to_string());
    }
    Ok(())
}

fn unix_epoch_millis() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock before unix epoch: {error}"))?;
    u64::try_from(duration.as_millis()).map_err(|_| "system time overflowed u64 millis".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tdw_protocol::{ActorKind, ActorRef, Op, SessionId};

    fn test_job(job_id: &str) -> WorkerJob {
        WorkerJob {
            job_id: job_id.to_string(),
            queue: "default".to_string(),
            envelope: OpEnvelope::new(
                SessionId::new("worker-session").expect("session id"),
                1,
                ActorRef {
                    actor_id: "worker-test".to_string(),
                    kind: ActorKind::Worker,
                    tenant_id: None,
                },
                Op::Shutdown,
            ),
            max_attempts: 3,
            not_before_ms: 0,
        }
    }

    #[test]
    fn worker_queue_contract_enqueues_leases_and_completes() {
        let mut queue = InMemoryWorkerQueue::default();
        queue.enqueue(test_job("job-1")).expect("enqueue");

        let lease = queue
            .lease_next("worker-1")
            .expect("lease")
            .expect("lease present");
        assert_eq!(lease.job_id, "job-1");
        assert_eq!(lease.worker_id, "worker-1");
        assert!(
            queue
                .lease_next("worker-2")
                .expect("second lease")
                .is_none()
        );

        queue.complete("job-1").expect("complete");
        assert!(queue.lease_next("worker-1").expect("lease").is_none());
    }

    #[test]
    fn worker_queue_respects_not_before_ms() {
        let mut queue = InMemoryWorkerQueue::default();
        let mut future = test_job("job-future");
        future.not_before_ms = unix_epoch_millis()
            .expect("system time")
            .saturating_add(60_000);
        queue.enqueue(future).expect("enqueue future");

        assert!(queue.lease_next("worker-1").expect("lease").is_none());

        queue.enqueue(test_job("job-ready")).expect("enqueue ready");
        let lease = queue
            .lease_next("worker-1")
            .expect("lease")
            .expect("ready lease");
        assert_eq!(lease.job_id, "job-ready");
    }

    #[test]
    fn worker_queue_contract_rejects_invalid_jobs() {
        let mut queue = InMemoryWorkerQueue::default();
        let mut invalid = test_job("");
        assert!(
            queue
                .enqueue(invalid.clone())
                .expect_err("empty id")
                .contains("job_id")
        );

        invalid.job_id = "job-2".to_string();
        invalid.max_attempts = 0;
        assert!(
            queue
                .enqueue(invalid)
                .expect_err("zero attempts")
                .contains("max_attempts")
        );

        assert!(
            queue
                .complete("missing")
                .expect_err("unknown complete")
                .contains("unknown job_id")
        );
    }
}
