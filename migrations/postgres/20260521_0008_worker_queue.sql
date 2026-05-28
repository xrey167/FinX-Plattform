create schema if not exists system;

create table if not exists system.worker_jobs (
  job_id text primary key,
  queue text not null,
  envelope_json jsonb not null,
  max_attempts bigint not null check (max_attempts > 0),
  not_before_ms bigint not null check (not_before_ms >= 0),
  priority integer not null,
  status text not null check (status in ('Pending', 'Leased', 'Completed', 'DeadLettered')),
  attempts bigint not null default 0 check (attempts >= 0),
  leased_by text,
  lease_expires_at_ms bigint,
  last_error text,
  created_at_ms bigint not null,
  updated_at_ms bigint not null,
  completed_at_ms bigint,
  dead_lettered_at_ms bigint
);

create index if not exists idx_worker_jobs_ready
  on system.worker_jobs(status, queue, not_before_ms, priority desc, created_at_ms, job_id);

create index if not exists idx_worker_jobs_expired_leases
  on system.worker_jobs(status, lease_expires_at_ms);
