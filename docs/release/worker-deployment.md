# Worker deployment - `PgWorkerQueue` rollout, supervision, and monitoring

Operational guide for running `tdw-worker`'s durable queue in production. It
closes follow-up #2 in
[`docs/quality/mcp-worker-product-boundaries.md`](../quality/mcp-worker-product-boundaries.md):
the durable queue is now driven by a supervised `tdw-worker --serve` lease loop
(`WorkerRunner`), and this page is its operating guide - the supervision and
monitoring you wrap around it, plus the one seam you must supply (the job
handler).

## What ships today vs. what you operate

Source of truth: `crates/tdw-worker/src/lib.rs` and `crates/tdw-worker/src/main.rs`.

| Capability | State |
|---|---|
| `PgWorkerQueue` (`--features postgres`): enqueue, priority lease, lease expiry/reap, retry, dead-letter, idempotent complete, stats | shipped (library) |
| SQLite durable scheduler + in-memory contract backend | shipped (library) |
| `tdw-worker --contract` (prints the queue contract) | shipped (CLI) |
| `tdw-worker --durable-smoke` (SQLite enqueue→lease→complete→stats) | shipped (CLI) |
| `tdw-worker --serve` (supervised lease loop, Ctrl-C drain) | shipped (CLI) |
| `tdw-worker --serve-once` (drain ready backlog, then exit) | shipped (CLI) |
| `WorkerRunner` + `JobHandler` (generic over the backend) | shipped (library) |
| Job business logic (the `JobHandler` impl) | **your code** - default is `LoggingAckHandler` |
| Metrics endpoint / alerting | **not shipped** - requirements below |

`tdw-worker --serve` runs the lease loop over a `SqliteWorkerQueue` (file path
from `TDW_WORKER_DB`, default `sqlite://tdw-worker.sqlite`) and drains in-flight
work on Ctrl-C. The one thing you supply is the `JobHandler` that executes each
job's `OpEnvelope`; the shipped default, `LoggingAckHandler`, acknowledges and
completes each job so the loop, retry, and dead-letter wiring can be exercised
end to end before real dispatch is plugged in. Tunables come from
`TDW_WORKER_ID`, `TDW_WORKER_LEASE_TTL_MS`, and `TDW_WORKER_POLL_MS`.

## Backend contract (the numbers you deploy against)

- **Connection:** `PgWorkerQueue::connect(database_url)` then `migrate()`, which
  creates `system.worker_jobs` (and its ready-index) with `IF NOT EXISTS`.
  Migration is idempotent and safe to run at process start.
- **Schema namespace:** Postgres jobs live in `system.worker_jobs`; the SQLite
  backend uses `worker_jobs`.
- **Lease TTL:** `DEFAULT_LEASE_TTL_MS = 30_000` (30s). Use
  `lease_next_with_ttl` to override per worker; pick a TTL comfortably larger
  than your slowest expected job step, or renew leases mid-job.
- **Priority lease order:** `(status, queue, not_before_ms, priority,
  created_at_ms)` - ready jobs, oldest-and-highest-priority first.
- **Retry / dead-letter:** each job carries `max_attempts`; once
  `attempts >= max_attempts` the job is dead-lettered with its last error.
- **Idempotency:** duplicate enqueue and repeated completion are no-ops, so a
  crash between "did the work" and "marked complete" does not double-process on
  replay.
- **Stats:** `WorkerQueueStats { ready, leased, dead_lettered, ... }` is the
  read model for monitoring.

## Reference lease loop

This is what `WorkerRunner` (behind `--serve`) does each iteration; it leases
the job *with its payload* via `lease_next_job_with_ttl`, so the handler gets
the full `OpEnvelope`:

```text
connect(TDW_WORKER_DB) -> migrate()
loop:
    if shutdown signaled (observed between jobs): stop
    leased = lease_next_job_with_ttl(worker_id, lease_ttl)
    if leased is None:
        reap_expired_leases(); sleep(poll_interval) or break on shutdown; continue
    match handler.handle(leased.job):       # your JobHandler runs the OpEnvelope
        Ok  => complete(leased.job.job_id)   # idempotent
        Err => fail(leased.job.job_id, err)  # increments attempts; dead-letters at max
```

In-flight jobs are never cancelled: the loop only observes the shutdown signal
between jobs and while idle, so a `SIGTERM`/Ctrl-C lets the current job finish
before the process exits.

Operational rules:

- **One `worker_id` per process instance.** Make it stable and unique (host +
  pid, or the orchestrator's task id) so leases and logs are attributable.
- **Renew or size the lease TTL** against the real job duration. A job that runs
  longer than the TTL will have its lease reaped and be re-leased elsewhere.
- **Call `reap_expired_leases` periodically** (or rely on the lease path, which
  reaps before leasing) so crashed-worker jobs return to `ready`.
- **Drain on shutdown:** stop accepting new leases on SIGTERM, finish in-flight
  work within the lease TTL, then exit. Anything not completed is reclaimed by
  expiry - safe because completion is idempotent.

## Process supervision

The lease loop must run under a supervisor that restarts it and bounds restart
storms:

- **systemd:** `Restart=always`, `RestartSec=5`, `StartLimitIntervalSec`/
  `StartLimitBurst` to cap crash loops, `TimeoutStopSec` >= lease TTL so drain
  completes, run as a dedicated non-root user.
- **Kubernetes:** a `Deployment` (not a `Job`); `terminationGracePeriodSeconds`
  >= lease TTL; liveness probe on the lease loop's heartbeat; readiness gated on
  a successful DB connection; `replicas` scaled by `ready` backlog. Leases make
  multiple replicas safe - no two workers hold the same job.
- **Connection pooling:** front Postgres with PgBouncer (transaction pooling) if
  you run many replicas; each `PgWorkerQueue` keeps its own pool.
- Run the container **unprivileged** with a read-only root filesystem; the
  worker needs only outbound DB connectivity and its secret.

## Monitoring and alerting

Drive these off `WorkerQueueStats` (polled on an interval) plus the dead-letter
table. Suggested signals:

| Metric | Source | Alert when |
|---|---|---|
| `worker_ready` (backlog) | `stats().ready` | sustained growth / above SLO threshold |
| `worker_leased` (in-flight) | `stats().leased` | stuck near 0 while `ready` is high (workers down) |
| `worker_dead_lettered` | `stats().dead_lettered` / `dead_letters()` | any increase (page on first dead-letter) |
| `lease_age` | `lease_expires_at_ms` vs. now | leases consistently near expiry (TTL too low) |
| `oldest_ready_age` | `created_at_ms` of oldest ready job | exceeds latency SLO |
| worker heartbeat | lease-loop liveness | missed -> restart / page |

- **Dead letters are the primary alarm.** A non-zero, rising
  `dead_lettered` means jobs exhausted `max_attempts`; route to an on-call queue
  and provide a replay/inspection path for `dead_letters()`.
- **Backlog vs. throughput.** Alert on `ready` rising while `leased` is flat -
  that is "no workers consuming," distinct from "too much work."
- Export the stats poll as Prometheus gauges (or your metrics system) from the
  worker process; do not query `system.worker_jobs` ad hoc from dashboards on a
  hot path.

## Rollout checklist

- [ ] `DATABASE_URL` provided from a secret manager; least-privilege role with
      access only to the `system` schema.
- [ ] `migrate()` runs at startup (idempotent); verified `system.worker_jobs`
      and its index exist.
- [ ] Lease TTL sized against the slowest job; drain-on-SIGTERM implemented.
- [ ] Supervisor configured with restart backoff and a stop timeout >= lease
      TTL.
- [ ] Stats exported as metrics; alerts wired for dead letters, backlog growth,
      and missed heartbeats.
- [ ] A documented replay/triage path for dead-lettered jobs.
- [ ] Load/restart tested: kill a worker mid-job and confirm the lease expires
      and the job is re-leased and completed exactly once.

## What this does NOT cover

- The job business logic. `--serve` ships with `LoggingAckHandler`, which
  acknowledges each job; wiring a `JobHandler` that dispatches the `OpEnvelope`
  (e.g. through `tdw-app-client` to a daemon) is the remaining integration step.
- A Postgres-backed `--serve` mode. The lease loop is generic over `ServeQueue`
  (both backends implement it), but the CLI currently serves the SQLite backend;
  selecting `PgWorkerQueue` from the CLI is a small follow-up.
- Cross-region or multi-cluster job routing.
- Exactly-once side effects in `run()`. The queue guarantees at-least-once
  delivery with idempotent completion; effect idempotency is the job's job.

## See also

- [`docs/quality/mcp-worker-product-boundaries.md`](../quality/mcp-worker-product-boundaries.md)
  - product boundary and the follow-up this page closes.
- [`docs/release/mcp-remote-deployment.md`](mcp-remote-deployment.md) - the
  companion remote MCP HTTP guide.
- [`docs/release/data-backend-runbook.md`](data-backend-runbook.md) - bringing
  Postgres up via the `live` compose profile.
- `crates/tdw-worker/src/lib.rs` - authoritative queue contract, lease TTL,
  schema, and stats shape.
