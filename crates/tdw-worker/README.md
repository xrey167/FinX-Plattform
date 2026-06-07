# tdw-worker

The TDW durable job worker. Provides a durable job queue with lease/retry/
dead-letter semantics (in-memory, SQLite, or Postgres backends) and a supervised
serve loop (`WorkerRunner`) that leases jobs and executes each one's `OpEnvelope`
by dispatching it to the TDW daemon. Bounded concurrency drives up to N jobs in
flight; in-flight jobs always run to completion on shutdown.

`#![forbid(unsafe_code)]`. The queue is idempotent on enqueue and complete,
times out leases, retries failed jobs up to `max_attempts`, then dead-letters
them (with list/replay).

## Binaries produced

- **`tdw-worker`** — the worker. Subcommands/flags:
  - `--serve` / `--serve-once` — run the lease loop (continuous / drain-once).
  - `dead-letters list` / `dead-letters replay <job_id>` — inspect/requeue.
  - `--durable-smoke` — offline SQLite enqueue/lease/complete self-check.
  - `--contract` — print the queue contract JSON.

## Feature flags

| Feature | Effect |
| --- | --- |
| `postgres` | enable the `PgWorkerQueue` distributed backend (`sqlx/postgres`) |

## Key environment variables

(Full reference in [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).)

- Backend: `TDW_WORKER_PG_URL` (or `DATABASE_URL`, needs `--features postgres`)
  selects Postgres; otherwise `TDW_WORKER_DB` (default
  `sqlite://tdw-worker.sqlite`).
- Serve loop: `TDW_WORKER_ID`, `TDW_WORKER_LEASE_TTL_MS`, `TDW_WORKER_POLL_MS`,
  `TDW_WORKER_CONCURRENCY` (default 4, clamped to `1..=256`).
- Daemon dispatch: `TDW_WORKER_DISPATCH=daemon`, `TDW_WORKER_DAEMON_ADDR`,
  `TDW_WORKER_DAEMON_TRANSPORT`, `TDW_WORKER_DAEMON_TIMEOUT_MS`. Without these the
  worker uses the offline ack handler.

## Quickstart (binary)

```bash
# Offline durable smoke (in-memory SQLite, no daemon, no network):
cargo run -p tdw-worker -- --durable-smoke

# Drain all ready jobs once against the default SQLite queue:
cargo run -p tdw-worker -- --serve-once

# Serve continuously, dispatching to a daemon over loopback TCP:
TDW_WORKER_DISPATCH=daemon TDW_WORKER_DAEMON_ADDR=127.0.0.1:7878 \
  cargo run -p tdw-worker -- --serve

# Inspect / replay dead letters:
cargo run -p tdw-worker -- dead-letters list
cargo run -p tdw-worker -- dead-letters replay <job_id>
```

See [`examples/basic.rs`](examples/basic.rs) for an offline in-memory queue demo
(enqueue → lease → complete + stats): `cargo run -p tdw-worker --example tdw_worker_basic`.

## See also

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — queue model, serve loop, daemon handler.
- `tdw-app-client` — the `DaemonClient` the `DaemonJobHandler` dispatches through.
- `tdw-protocol` — the `OpEnvelope` carried as the job payload.
