# tdw-worker architecture

`tdw-worker` is a durable job queue plus a supervised lease loop. A job carries an
`OpEnvelope`; the worker leases it, runs a `JobHandler` (in production: dispatch
to the daemon), and completes or fails it with retry/dead-letter semantics.

## Module map

| Path | Contents |
| --- | --- |
| `src/lib.rs` | queue types, the three backends, the `ServeQueue`/`JobHandler` traits, `WorkerRunner`, the daemon handler |
| `src/main.rs` | the `tdw-worker` binary: `--serve`/`--serve-once`, `dead-letters`, smokes, env config |

## Queue model

A `WorkerJob { job_id, queue, envelope, max_attempts, not_before_ms, priority }`
moves through `WorkerJobStatus`: `Pending → Leased → Completed`, or
`Pending → … → DeadLettered`. Core operations (the `DurableWorkerQueue` /
backend surface):

- `enqueue` — idempotent on `job_id` (`EnqueueOutcome.inserted` is false on
  conflict).
- `lease_next[_job][_with_ttl]` — pick the highest-priority ready job
  (`status = Pending`, `not_before_ms <= now`), increment `attempts`, set a lease
  expiry. The `*_job` variants return the full payload (`LeasedJob`).
- `complete` — idempotent terminal success.
- `fail(job_id, error, retry_after_ms)` — retry (`Pending`, delayed) until
  `attempts >= max_attempts`, then `DeadLettered`.
- `reap_expired_leases` — requeue (or dead-letter) jobs whose lease lapsed.
- `dead_letters` / `replay_dead_letter` — list / reset-to-`Pending`.
- `stats` — counts by status.

### Backends

- `InMemoryWorkerQueue` — the contract reference (sync, no persistence).
- `SqliteWorkerQueue` — durable single-node (`worker_jobs` table +
  `SQLITE_WORKER_MIGRATION`).
- `PgWorkerQueue` (feature `postgres`) — distributed; leasing uses
  `for update skip locked` so many workers contend safely.

The Postgres and SQLite backends both implement the async `ServeQueue` trait, so
`WorkerRunner` is generic over the backend.

## Serve loop (`WorkerRunner<Q, H>`)

`WorkerRunner::new(queue, handler, ServeConfig)` drives a `ServeQueue` with a
`JobHandler`:

- `run_until_idle()` — drain every ready job (up to `max_concurrent` in flight
  via a `FuturesUnordered`), reap expired leases, return a `ServeReport`
  (processed/completed/failed/dead_lettered). Used by `--serve-once`.
- `run(shutdown)` — lease loop until `shutdown` resolves; observing shutdown stops
  new leases and then drains in-flight jobs to completion — a stop signal never
  cancels already-leased work. Used by `--serve`.

`ServeConfig { worker_id, lease_ttl_ms, poll_interval_ms, max_concurrent }`
(default concurrency 4). The binary's `clamp_concurrency` bounds
`TDW_WORKER_CONCURRENCY` to `1..=256`.

### Handlers

- `LoggingAckHandler` — offline default: acknowledges each job (no side effects).
- `DaemonJobHandler` — production: submits the job's `OpEnvelope` via a
  `tdw_app_client::DaemonClient` on a `spawn_blocking` task (the client is
  blocking std I/O), then maps the daemon's terminal event onto the job result
  (`terminal_outcome`: `Completed → Ok`; `Failed`/`Cancelled`/non-terminal/no
  event → `Err`, which the runner retries then dead-letters).

## Runtime flow (worker lease loop)

```text
queue (SQLite | Postgres)
   └─▶ WorkerRunner.run(shutdown)
          lease_next_job_with_ttl (up to max_concurrent in flight)
             └─▶ JobHandler.handle(job)
                    DaemonJobHandler: spawn_blocking → DaemonClient.submit_and_wait
                                      → terminal EventMsg → Ok/Err
          Ok  → queue.complete(job_id)
          Err → queue.fail(job_id, err, 0)  (retry or DeadLetter at max_attempts)
          idle → reap_expired_leases ; sleep(poll_interval) ; repeat
          shutdown → stop leasing, drain in-flight, return ServeReport
```

## Security posture

- The worker holds **no credentials**; it dispatches to the daemon, which
  enforces policy. The daemon endpoint is validated (`DaemonClientConfig::validate`)
  and fails closed on unsupported transports (e.g. UDS on Windows).
- **Bounded concurrency**: `TDW_WORKER_CONCURRENCY` is clamped to `1..=256` so a
  typo cannot exhaust DB connections / file descriptors.
- **At-least-once with idempotency**: leases time out and retry; `enqueue` and
  `complete` are idempotent; dead-letters are explicit and replayable. Persisted
  integer fields are range-checked on read (`InvalidPersistedValue`).
- The daemon dispatch I/O runs on a blocking task so the loop's timers/signals
  stay responsive.

## Integration points

- `tdw-app-client` — `DaemonClient`/`DaemonClientConfig`/`DaemonSubmission`.
- `tdw-protocol` — `OpEnvelope`/`EventMsg` (job payload + terminal mapping).
- `sqlx` — SQLite/Postgres backends; the Postgres migration is vendored from
  `migrations/postgres/`.
