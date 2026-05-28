#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Row, SqlitePool};
use std::time::{SystemTime, UNIX_EPOCH};
use tdw_protocol::{ActorKind, ActorRef, Op, OpEnvelope, SessionId};
use thiserror::Error;

const DEFAULT_LEASE_TTL_MS: u64 = 30_000;

pub type Result<T> = std::result::Result<T, WorkerQueueError>;

#[derive(Debug, Error)]
pub enum WorkerQueueError {
    #[error("invalid job: {0}")]
    InvalidJob(String),
    #[error("invalid worker: {0}")]
    InvalidWorker(String),
    #[error("duplicate job_id: {0}")]
    DuplicateJob(String),
    #[error("unknown job_id: {0}")]
    UnknownJob(String),
    #[error("invalid persisted status: {0}")]
    InvalidStatus(String),
    #[error("invalid persisted value for {field}: {value}")]
    InvalidPersistedValue { field: &'static str, value: i64 },
    #[error("system clock before unix epoch: {0}")]
    Clock(String),
    #[error("system time overflowed i64 milliseconds")]
    TimeOverflow,
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerJobStatus {
    Pending,
    Leased,
    Completed,
    DeadLettered,
}

impl WorkerJobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Leased => "Leased",
            Self::Completed => "Completed",
            Self::DeadLettered => "DeadLettered",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "Pending" => Ok(Self::Pending),
            "Leased" => Ok(Self::Leased),
            "Completed" => Ok(Self::Completed),
            "DeadLettered" => Ok(Self::DeadLettered),
            other => Err(WorkerQueueError::InvalidStatus(other.to_string())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerJob {
    pub job_id: String,
    pub queue: String,
    pub envelope: OpEnvelope,
    pub max_attempts: u32,
    pub not_before_ms: u64,
    pub priority: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkerLease {
    pub job_id: String,
    pub worker_id: String,
    pub attempt: u32,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnqueueOutcome {
    pub job_id: String,
    pub inserted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeadLetterRecord {
    pub job: WorkerJob,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub dead_lettered_at_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerQueueStats {
    pub pending: u64,
    pub leased: u64,
    pub completed: u64,
    pub dead_lettered: u64,
}

pub trait DurableWorkerQueue {
    fn enqueue(&mut self, job: WorkerJob) -> Result<EnqueueOutcome>;

    fn lease_next(&mut self, worker_id: &str) -> Result<Option<WorkerLease>> {
        self.lease_next_with_ttl(worker_id, DEFAULT_LEASE_TTL_MS)
    }

    fn lease_next_with_ttl(
        &mut self,
        worker_id: &str,
        lease_ttl_ms: u64,
    ) -> Result<Option<WorkerLease>>;

    fn complete(&mut self, job_id: &str) -> Result<()>;
    fn fail(&mut self, job_id: &str, error: &str, retry_after_ms: u64) -> Result<WorkerJobStatus>;
    fn reap_expired_leases(&mut self) -> Result<u64>;
    fn dead_letters(&self) -> Vec<DeadLetterRecord>;
    fn stats(&self) -> WorkerQueueStats;
}

#[derive(Default)]
pub struct InMemoryWorkerQueue {
    records: Vec<InMemoryWorkerRecord>,
}

#[derive(Clone)]
struct InMemoryWorkerRecord {
    job: WorkerJob,
    status: WorkerJobStatus,
    attempts: u32,
    leased_by: Option<String>,
    lease_expires_at_ms: Option<u64>,
    last_error: Option<String>,
    dead_lettered_at_ms: Option<u64>,
}

impl DurableWorkerQueue for InMemoryWorkerQueue {
    fn enqueue(&mut self, job: WorkerJob) -> Result<EnqueueOutcome> {
        validate_job(&job)?;
        if self
            .records
            .iter()
            .any(|existing| existing.job.job_id == job.job_id)
        {
            return Err(WorkerQueueError::DuplicateJob(job.job_id));
        }

        let job_id = job.job_id.clone();
        self.records.push(InMemoryWorkerRecord {
            job,
            status: WorkerJobStatus::Pending,
            attempts: 0,
            leased_by: None,
            lease_expires_at_ms: None,
            last_error: None,
            dead_lettered_at_ms: None,
        });
        Ok(EnqueueOutcome {
            job_id,
            inserted: true,
        })
    }

    fn lease_next_with_ttl(
        &mut self,
        worker_id: &str,
        lease_ttl_ms: u64,
    ) -> Result<Option<WorkerLease>> {
        validate_worker_id(worker_id)?;
        let now_ms = unix_epoch_millis()?;
        self.reap_expired_leases_at(now_ms)?;

        let Some(index) = self
            .records
            .iter()
            .enumerate()
            .filter(|(_, record)| {
                record.status == WorkerJobStatus::Pending && record.job.not_before_ms <= now_ms
            })
            .min_by(|(_, left), (_, right)| {
                right
                    .job
                    .priority
                    .cmp(&left.job.priority)
                    .then(left.job.not_before_ms.cmp(&right.job.not_before_ms))
                    .then(left.job.job_id.cmp(&right.job.job_id))
            })
            .map(|(index, _)| index)
        else {
            return Ok(None);
        };

        let record = &mut self.records[index];
        record.status = WorkerJobStatus::Leased;
        record.attempts = record.attempts.saturating_add(1);
        record.leased_by = Some(worker_id.to_string());
        record.lease_expires_at_ms = Some(now_ms.saturating_add(lease_ttl_ms));
        Ok(Some(WorkerLease {
            job_id: record.job.job_id.clone(),
            worker_id: worker_id.to_string(),
            attempt: record.attempts,
            lease_expires_at_ms: record.lease_expires_at_ms.unwrap_or(now_ms),
        }))
    }

    fn complete(&mut self, job_id: &str) -> Result<()> {
        let record = self.record_mut(job_id)?;
        if record.status == WorkerJobStatus::Completed {
            return Ok(());
        }
        record.status = WorkerJobStatus::Completed;
        record.leased_by = None;
        record.lease_expires_at_ms = None;
        Ok(())
    }

    fn fail(&mut self, job_id: &str, error: &str, retry_after_ms: u64) -> Result<WorkerJobStatus> {
        let now_ms = unix_epoch_millis()?;
        let record = self.record_mut(job_id)?;
        record.last_error = Some(error.to_string());
        record.leased_by = None;
        record.lease_expires_at_ms = None;

        if record.attempts >= record.job.max_attempts {
            record.status = WorkerJobStatus::DeadLettered;
            record.dead_lettered_at_ms = Some(now_ms);
            return Ok(WorkerJobStatus::DeadLettered);
        }

        record.status = WorkerJobStatus::Pending;
        record.job.not_before_ms = now_ms.saturating_add(retry_after_ms);
        Ok(WorkerJobStatus::Pending)
    }

    fn reap_expired_leases(&mut self) -> Result<u64> {
        let now_ms = unix_epoch_millis()?;
        self.reap_expired_leases_at(now_ms)
    }

    fn dead_letters(&self) -> Vec<DeadLetterRecord> {
        self.records
            .iter()
            .filter(|record| record.status == WorkerJobStatus::DeadLettered)
            .filter_map(|record| {
                Some(DeadLetterRecord {
                    job: record.job.clone(),
                    attempts: record.attempts,
                    last_error: record.last_error.clone(),
                    dead_lettered_at_ms: record.dead_lettered_at_ms?,
                })
            })
            .collect()
    }

    fn stats(&self) -> WorkerQueueStats {
        self.records
            .iter()
            .fold(WorkerQueueStats::default(), |mut stats, record| {
                match record.status {
                    WorkerJobStatus::Pending => stats.pending += 1,
                    WorkerJobStatus::Leased => stats.leased += 1,
                    WorkerJobStatus::Completed => stats.completed += 1,
                    WorkerJobStatus::DeadLettered => stats.dead_lettered += 1,
                }
                stats
            })
    }
}

impl InMemoryWorkerQueue {
    fn record_mut(&mut self, job_id: &str) -> Result<&mut InMemoryWorkerRecord> {
        self.records
            .iter_mut()
            .find(|record| record.job.job_id == job_id)
            .ok_or_else(|| WorkerQueueError::UnknownJob(job_id.to_string()))
    }

    fn reap_expired_leases_at(&mut self, now_ms: u64) -> Result<u64> {
        let mut reaped = 0;
        for record in &mut self.records {
            if record.status != WorkerJobStatus::Leased {
                continue;
            }
            let Some(expires_at) = record.lease_expires_at_ms else {
                continue;
            };
            if expires_at > now_ms {
                continue;
            }

            reaped += 1;
            record.leased_by = None;
            record.lease_expires_at_ms = None;
            if record.attempts >= record.job.max_attempts {
                record.status = WorkerJobStatus::DeadLettered;
                record.dead_lettered_at_ms = Some(now_ms);
                if record.last_error.is_none() {
                    record.last_error = Some("lease expired".to_string());
                }
            } else {
                record.status = WorkerJobStatus::Pending;
            }
        }
        Ok(reaped)
    }
}

#[derive(Clone)]
pub struct SqliteWorkerQueue {
    pool: SqlitePool,
}

impl SqliteWorkerQueue {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let options = database_url
            .parse::<SqliteConnectOptions>()?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let queue = Self { pool };
        queue.migrate().await?;
        Ok(queue)
    }

    pub fn from_pool(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> Result<()> {
        for statement in SQLITE_WORKER_MIGRATION
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn enqueue(&self, job: WorkerJob) -> Result<EnqueueOutcome> {
        validate_job(&job)?;
        let now_ms = unix_epoch_millis()?;
        let envelope_json = serde_json::to_string(&job.envelope)?;
        let result = sqlx::query(
            r#"
            insert into worker_jobs (
                job_id, queue, envelope_json, max_attempts, not_before_ms, priority,
                status, attempts, created_at_ms, updated_at_ms
            )
            values (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?8)
            on conflict(job_id) do nothing
            "#,
        )
        .bind(&job.job_id)
        .bind(&job.queue)
        .bind(envelope_json)
        .bind(u32_to_i64(job.max_attempts))
        .bind(u64_to_i64(job.not_before_ms)?)
        .bind(i64::from(job.priority))
        .bind(WorkerJobStatus::Pending.as_str())
        .bind(u64_to_i64(now_ms)?)
        .execute(&self.pool)
        .await?;

        Ok(EnqueueOutcome {
            job_id: job.job_id,
            inserted: result.rows_affected() == 1,
        })
    }

    pub async fn lease_next(&self, worker_id: &str) -> Result<Option<WorkerLease>> {
        self.lease_next_with_ttl(worker_id, DEFAULT_LEASE_TTL_MS)
            .await
    }

    pub async fn lease_next_with_ttl(
        &self,
        worker_id: &str,
        lease_ttl_ms: u64,
    ) -> Result<Option<WorkerLease>> {
        validate_worker_id(worker_id)?;
        let now_ms = unix_epoch_millis()?;
        self.reap_expired_leases_at(now_ms).await?;
        let lease_expires_at_ms = now_ms.saturating_add(lease_ttl_ms);

        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            r#"
            select job_id, attempts
            from worker_jobs
            where status = ?1 and not_before_ms <= ?2
            order by priority desc, not_before_ms asc, created_at_ms asc, job_id asc
            limit 1
            "#,
        )
        .bind(WorkerJobStatus::Pending.as_str())
        .bind(u64_to_i64(now_ms)?)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };

        let job_id: String = row.get("job_id");
        let attempts = i64_to_u32(row.get("attempts"), "attempts")?;
        let attempt = attempts.saturating_add(1);
        let update = sqlx::query(
            r#"
            update worker_jobs
            set status = ?1,
                attempts = attempts + 1,
                leased_by = ?2,
                lease_expires_at_ms = ?3,
                updated_at_ms = ?4
            where job_id = ?5 and status = ?6
            "#,
        )
        .bind(WorkerJobStatus::Leased.as_str())
        .bind(worker_id)
        .bind(u64_to_i64(lease_expires_at_ms)?)
        .bind(u64_to_i64(now_ms)?)
        .bind(&job_id)
        .bind(WorkerJobStatus::Pending.as_str())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        if update.rows_affected() == 0 {
            return Ok(None);
        }

        Ok(Some(WorkerLease {
            job_id,
            worker_id: worker_id.to_string(),
            attempt,
            lease_expires_at_ms,
        }))
    }

    pub async fn complete(&self, job_id: &str) -> Result<()> {
        let Some(status) = self.job_status(job_id).await? else {
            return Err(WorkerQueueError::UnknownJob(job_id.to_string()));
        };
        if status == WorkerJobStatus::Completed {
            return Ok(());
        }

        let now_ms = unix_epoch_millis()?;
        sqlx::query(
            r#"
            update worker_jobs
            set status = ?1,
                leased_by = null,
                lease_expires_at_ms = null,
                completed_at_ms = ?2,
                updated_at_ms = ?2
            where job_id = ?3
            "#,
        )
        .bind(WorkerJobStatus::Completed.as_str())
        .bind(u64_to_i64(now_ms)?)
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn fail(
        &self,
        job_id: &str,
        error: &str,
        retry_after_ms: u64,
    ) -> Result<WorkerJobStatus> {
        let row =
            sqlx::query("select attempts, max_attempts from worker_jobs where job_id = ?1 limit 1")
                .bind(job_id)
                .fetch_optional(&self.pool)
                .await?;
        let Some(row) = row else {
            return Err(WorkerQueueError::UnknownJob(job_id.to_string()));
        };

        let attempts = i64_to_u32(row.get("attempts"), "attempts")?;
        let max_attempts = i64_to_u32(row.get("max_attempts"), "max_attempts")?;
        let now_ms = unix_epoch_millis()?;
        if attempts >= max_attempts {
            self.dead_letter(job_id, error, now_ms).await?;
            return Ok(WorkerJobStatus::DeadLettered);
        }

        let not_before_ms = now_ms.saturating_add(retry_after_ms);
        sqlx::query(
            r#"
            update worker_jobs
            set status = ?1,
                not_before_ms = ?2,
                leased_by = null,
                lease_expires_at_ms = null,
                last_error = ?3,
                updated_at_ms = ?4
            where job_id = ?5
            "#,
        )
        .bind(WorkerJobStatus::Pending.as_str())
        .bind(u64_to_i64(not_before_ms)?)
        .bind(error)
        .bind(u64_to_i64(now_ms)?)
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(WorkerJobStatus::Pending)
    }

    pub async fn reap_expired_leases(&self) -> Result<u64> {
        let now_ms = unix_epoch_millis()?;
        self.reap_expired_leases_at(now_ms).await
    }

    pub async fn reap_expired_leases_at(&self, now_ms: u64) -> Result<u64> {
        let now_i64 = u64_to_i64(now_ms)?;
        let dead_lettered = sqlx::query(
            r#"
            update worker_jobs
            set status = ?1,
                leased_by = null,
                lease_expires_at_ms = null,
                last_error = coalesce(last_error, 'lease expired'),
                dead_lettered_at_ms = ?2,
                updated_at_ms = ?2
            where status = ?3
              and lease_expires_at_ms is not null
              and lease_expires_at_ms <= ?2
              and attempts >= max_attempts
            "#,
        )
        .bind(WorkerJobStatus::DeadLettered.as_str())
        .bind(now_i64)
        .bind(WorkerJobStatus::Leased.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();

        let requeued = sqlx::query(
            r#"
            update worker_jobs
            set status = ?1,
                leased_by = null,
                lease_expires_at_ms = null,
                updated_at_ms = ?2
            where status = ?3
              and lease_expires_at_ms is not null
              and lease_expires_at_ms <= ?2
              and attempts < max_attempts
            "#,
        )
        .bind(WorkerJobStatus::Pending.as_str())
        .bind(now_i64)
        .bind(WorkerJobStatus::Leased.as_str())
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(dead_lettered + requeued)
    }

    pub async fn dead_letters(&self) -> Result<Vec<DeadLetterRecord>> {
        let rows = sqlx::query(
            r#"
            select job_id, queue, envelope_json, max_attempts, not_before_ms, priority,
                   attempts, last_error, dead_lettered_at_ms
            from worker_jobs
            where status = ?1
            order by dead_lettered_at_ms asc, job_id asc
            "#,
        )
        .bind(WorkerJobStatus::DeadLettered.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(row_to_dead_letter).collect()
    }

    pub async fn job_status(&self, job_id: &str) -> Result<Option<WorkerJobStatus>> {
        let row = sqlx::query("select status from worker_jobs where job_id = ?1 limit 1")
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| WorkerJobStatus::parse(row.get::<&str, _>("status")))
            .transpose()
    }

    pub async fn stats(&self) -> Result<WorkerQueueStats> {
        let rows = sqlx::query(
            r#"
            select status, count(*) as count
            from worker_jobs
            group by status
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut stats = WorkerQueueStats::default();
        for row in rows {
            let status = WorkerJobStatus::parse(row.get::<&str, _>("status"))?;
            let count = i64_to_u64(row.get("count"), "count")?;
            match status {
                WorkerJobStatus::Pending => stats.pending = count,
                WorkerJobStatus::Leased => stats.leased = count,
                WorkerJobStatus::Completed => stats.completed = count,
                WorkerJobStatus::DeadLettered => stats.dead_lettered = count,
            }
        }
        Ok(stats)
    }

    async fn dead_letter(&self, job_id: &str, error: &str, now_ms: u64) -> Result<()> {
        sqlx::query(
            r#"
            update worker_jobs
            set status = ?1,
                leased_by = null,
                lease_expires_at_ms = null,
                last_error = ?2,
                dead_lettered_at_ms = ?3,
                updated_at_ms = ?3
            where job_id = ?4
            "#,
        )
        .bind(WorkerJobStatus::DeadLettered.as_str())
        .bind(error)
        .bind(u64_to_i64(now_ms)?)
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

pub const SQLITE_WORKER_MIGRATION: &str = r#"
create table if not exists worker_jobs (
    job_id text primary key,
    queue text not null,
    envelope_json text not null,
    max_attempts integer not null,
    not_before_ms integer not null,
    priority integer not null,
    status text not null,
    attempts integer not null,
    leased_by text,
    lease_expires_at_ms integer,
    last_error text,
    created_at_ms integer not null,
    updated_at_ms integer not null,
    completed_at_ms integer,
    dead_lettered_at_ms integer
);
create index if not exists idx_worker_jobs_ready
    on worker_jobs(status, queue, not_before_ms, priority, created_at_ms);
create index if not exists idx_worker_jobs_expired_leases
    on worker_jobs(status, lease_expires_at_ms);
"#;

pub fn worker_contract_json() -> String {
    serde_json::json!({
        "contract": "tdw.worker.queue.v1",
        "job": {
            "job_id": "stable idempotency key",
            "queue": "logical queue name",
            "envelope": "tdw_protocol::OpEnvelope",
            "max_attempts": "positive integer",
            "not_before_ms": "unix epoch milliseconds",
            "priority": "higher values lease first"
        },
        "operations": [
            "enqueue",
            "lease_next",
            "complete",
            "fail",
            "reap_expired_leases",
            "dead_letters",
            "stats"
        ],
        "backends": ["in_memory_contract", "sqlite_durable"],
        "durability": {
            "lease_timeout": true,
            "retry_counter": true,
            "dead_letter": true,
            "idempotent_enqueue": true,
            "idempotent_complete": true
        }
    })
    .to_string()
}

pub fn sample_shutdown_job(job_id: &str) -> Result<WorkerJob> {
    Ok(WorkerJob {
        job_id: job_id.to_string(),
        queue: "default".to_string(),
        envelope: OpEnvelope::new(
            SessionId::new("worker-session")
                .map_err(|error| WorkerQueueError::InvalidJob(error.to_string()))?,
            1,
            ActorRef {
                actor_id: "worker-smoke".to_string(),
                kind: ActorKind::Worker,
                tenant_id: None,
            },
            Op::Shutdown,
        ),
        max_attempts: 3,
        not_before_ms: 0,
        priority: 0,
    })
}

fn validate_job(job: &WorkerJob) -> Result<()> {
    if job.job_id.trim().is_empty() {
        return Err(WorkerQueueError::InvalidJob(
            "job_id must not be empty".to_string(),
        ));
    }
    if job.queue.trim().is_empty() {
        return Err(WorkerQueueError::InvalidJob(
            "queue must not be empty".to_string(),
        ));
    }
    if job.max_attempts == 0 {
        return Err(WorkerQueueError::InvalidJob(
            "max_attempts must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

fn validate_worker_id(worker_id: &str) -> Result<()> {
    if worker_id.trim().is_empty() {
        return Err(WorkerQueueError::InvalidWorker(
            "worker_id must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn unix_epoch_millis() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| WorkerQueueError::Clock(error.to_string()))?;
    u64::try_from(duration.as_millis()).map_err(|_| WorkerQueueError::TimeOverflow)
}

fn u32_to_i64(value: u32) -> i64 {
    i64::from(value)
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| WorkerQueueError::TimeOverflow)
}

fn i64_to_u64(value: i64, field: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| WorkerQueueError::InvalidPersistedValue { field, value })
}

fn i64_to_u32(value: i64, field: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| WorkerQueueError::InvalidPersistedValue { field, value })
}

fn row_to_job(row: &SqliteRow) -> Result<WorkerJob> {
    Ok(WorkerJob {
        job_id: row.get("job_id"),
        queue: row.get("queue"),
        envelope: serde_json::from_str(row.get::<&str, _>("envelope_json"))?,
        max_attempts: i64_to_u32(row.get("max_attempts"), "max_attempts")?,
        not_before_ms: i64_to_u64(row.get("not_before_ms"), "not_before_ms")?,
        priority: i32::try_from(row.get::<i64, _>("priority")).map_err(|_| {
            WorkerQueueError::InvalidPersistedValue {
                field: "priority",
                value: row.get("priority"),
            }
        })?,
    })
}

fn row_to_dead_letter(row: SqliteRow) -> Result<DeadLetterRecord> {
    Ok(DeadLetterRecord {
        job: row_to_job(&row)?,
        attempts: i64_to_u32(row.get("attempts"), "attempts")?,
        last_error: row.get("last_error"),
        dead_lettered_at_ms: i64_to_u64(row.get("dead_lettered_at_ms"), "dead_lettered_at_ms")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_job(job_id: &str) -> WorkerJob {
        sample_shutdown_job(job_id).unwrap_or_else(|error| panic!("sample job builds: {error}"))
    }

    fn sqlite_file_url(name: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tdw-worker-{name}-{nanos}.sqlite"));
        format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
    }

    #[test]
    fn worker_queue_contract_enqueues_leases_and_completes() {
        let mut queue = InMemoryWorkerQueue::default();
        let outcome = queue.enqueue(test_job("job-1")).expect("enqueue");
        assert!(outcome.inserted);

        let lease = queue
            .lease_next("worker-1")
            .expect("lease")
            .expect("lease present");
        assert_eq!(lease.job_id, "job-1");
        assert_eq!(lease.worker_id, "worker-1");
        assert_eq!(lease.attempt, 1);
        assert!(
            queue
                .lease_next("worker-2")
                .expect("second lease")
                .is_none()
        );

        queue.complete("job-1").expect("complete");
        queue.complete("job-1").expect("idempotent complete");
        assert!(queue.lease_next("worker-1").expect("lease").is_none());
        assert_eq!(queue.stats().completed, 1);
    }

    #[test]
    fn worker_queue_respects_not_before_ms_and_priority() {
        let mut queue = InMemoryWorkerQueue::default();
        let mut future = test_job("job-future");
        future.not_before_ms = unix_epoch_millis()
            .expect("system time")
            .saturating_add(60_000);
        queue.enqueue(future).expect("enqueue future");

        let mut low = test_job("job-low");
        low.priority = 1;
        queue.enqueue(low).expect("enqueue low");

        let mut high = test_job("job-high");
        high.priority = 10;
        queue.enqueue(high).expect("enqueue high");

        let lease = queue
            .lease_next("worker-1")
            .expect("lease")
            .expect("ready lease");
        assert_eq!(lease.job_id, "job-high");
    }

    #[test]
    fn worker_queue_retries_and_dead_letters_after_expired_leases() {
        let mut queue = InMemoryWorkerQueue::default();
        let mut job = test_job("job-retry");
        job.max_attempts = 2;
        queue.enqueue(job).expect("enqueue");

        let first = queue
            .lease_next_with_ttl("worker-1", 0)
            .expect("first lease")
            .expect("first lease present");
        assert_eq!(first.attempt, 1);
        assert_eq!(queue.reap_expired_leases().expect("reap"), 1);
        assert_eq!(queue.stats().pending, 1);

        let second = queue
            .lease_next_with_ttl("worker-2", 0)
            .expect("second lease")
            .expect("second lease present");
        assert_eq!(second.attempt, 2);
        assert_eq!(queue.reap_expired_leases().expect("reap"), 1);
        assert_eq!(queue.stats().dead_lettered, 1);
        assert_eq!(queue.dead_letters()[0].job.job_id, "job-retry");
    }

    #[test]
    fn worker_queue_contract_rejects_invalid_jobs() {
        let mut queue = InMemoryWorkerQueue::default();
        let mut invalid = test_job("");
        assert!(
            queue
                .enqueue(invalid.clone())
                .expect_err("empty id")
                .to_string()
                .contains("job_id")
        );

        invalid.job_id = "job-2".to_string();
        invalid.max_attempts = 0;
        assert!(
            queue
                .enqueue(invalid)
                .expect_err("zero attempts")
                .to_string()
                .contains("max_attempts")
        );

        assert!(
            queue
                .complete("missing")
                .expect_err("unknown complete")
                .to_string()
                .contains("unknown job_id")
        );
    }

    #[tokio::test]
    async fn sqlite_queue_persists_jobs_across_reconnect() {
        let database_url = sqlite_file_url("persist");
        let first = SqliteWorkerQueue::connect(&database_url)
            .await
            .unwrap_or_else(|error| panic!("connect first: {error}"));
        let outcome = first
            .enqueue(test_job("job-persist"))
            .await
            .unwrap_or_else(|error| panic!("enqueue: {error}"));
        assert!(outcome.inserted);
        drop(first);

        let second = SqliteWorkerQueue::connect(&database_url)
            .await
            .unwrap_or_else(|error| panic!("connect second: {error}"));
        let lease = second
            .lease_next("worker-1")
            .await
            .unwrap_or_else(|error| panic!("lease: {error}"))
            .expect("lease present");
        assert_eq!(lease.job_id, "job-persist");
        second
            .complete("job-persist")
            .await
            .unwrap_or_else(|error| panic!("complete: {error}"));
        assert_eq!(
            second
                .job_status("job-persist")
                .await
                .unwrap_or_else(|error| panic!("status: {error}")),
            Some(WorkerJobStatus::Completed)
        );
    }

    #[tokio::test]
    async fn sqlite_queue_idempotent_enqueue_and_priority_leasing() {
        let queue = SqliteWorkerQueue::connect("sqlite::memory:")
            .await
            .unwrap_or_else(|error| panic!("connect: {error}"));
        let mut low = test_job("job-low");
        low.priority = 1;
        let mut high = test_job("job-high");
        high.priority = 20;

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
                .unwrap_or_else(|error| panic!("idempotent enqueue: {error}"))
                .inserted
        );
        queue
            .enqueue(high)
            .await
            .unwrap_or_else(|error| panic!("enqueue high: {error}"));

        let lease = queue
            .lease_next("worker-1")
            .await
            .unwrap_or_else(|error| panic!("lease: {error}"))
            .expect("lease present");
        assert_eq!(lease.job_id, "job-high");
    }

    #[tokio::test]
    async fn sqlite_queue_retries_and_dead_letters_after_expired_leases() {
        let queue = SqliteWorkerQueue::connect("sqlite::memory:")
            .await
            .unwrap_or_else(|error| panic!("connect: {error}"));
        let mut job = test_job("job-dead");
        job.max_attempts = 2;
        queue
            .enqueue(job)
            .await
            .unwrap_or_else(|error| panic!("enqueue: {error}"));

        let first = queue
            .lease_next_with_ttl("worker-1", 0)
            .await
            .unwrap_or_else(|error| panic!("first lease: {error}"))
            .expect("first lease present");
        assert_eq!(first.attempt, 1);
        assert_eq!(
            queue
                .reap_expired_leases()
                .await
                .unwrap_or_else(|error| panic!("reap first: {error}")),
            1
        );
        assert_eq!(
            queue
                .stats()
                .await
                .unwrap_or_else(|error| panic!("stats first: {error}"))
                .pending,
            1
        );

        let second = queue
            .lease_next_with_ttl("worker-2", 0)
            .await
            .unwrap_or_else(|error| panic!("second lease: {error}"))
            .expect("second lease present");
        assert_eq!(second.attempt, 2);
        assert_eq!(
            queue
                .reap_expired_leases()
                .await
                .unwrap_or_else(|error| panic!("reap second: {error}")),
            1
        );
        let letters = queue
            .dead_letters()
            .await
            .unwrap_or_else(|error| panic!("dead letters: {error}"));
        assert_eq!(letters.len(), 1);
        assert_eq!(letters[0].job.job_id, "job-dead");
        assert_eq!(letters[0].attempts, 2);
    }

    #[tokio::test]
    async fn sqlite_queue_fail_retries_until_max_attempts() {
        let queue = SqliteWorkerQueue::connect("sqlite::memory:")
            .await
            .unwrap_or_else(|error| panic!("connect: {error}"));
        let mut job = test_job("job-fail");
        job.max_attempts = 1;
        queue
            .enqueue(job)
            .await
            .unwrap_or_else(|error| panic!("enqueue: {error}"));
        queue
            .lease_next("worker-1")
            .await
            .unwrap_or_else(|error| panic!("lease: {error}"))
            .expect("lease present");

        let status = queue
            .fail("job-fail", "boom", 0)
            .await
            .unwrap_or_else(|error| panic!("fail: {error}"));
        assert_eq!(status, WorkerJobStatus::DeadLettered);
        let letters = queue
            .dead_letters()
            .await
            .unwrap_or_else(|error| panic!("dead letters: {error}"));
        assert_eq!(letters[0].last_error.as_deref(), Some("boom"));
    }
}
