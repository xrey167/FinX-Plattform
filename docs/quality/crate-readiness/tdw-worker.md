# tdw-worker Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-worker\Cargo.toml
- Target kinds: lib, bin
- Local dependencies: tdw-protocol, tdw-service-api
- External dependencies: serde, serde_json, sqlx, thiserror, tokio
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: `postgres` enables the Postgres/RiverQueue-style distributed
  scheduler backend while keeping the default workspace test set offline.
- Test attributes detected: tokio async tests and unit tests in src/lib.rs
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 51 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed.
- [x] Feature flags reviewed or marked not applicable.
- [x] Public API and error contracts reviewed.
- [x] Runtime behavior reviewed.
- [x] Tests and coverage evidence recorded.
- [x] Docs and examples reviewed.
- [x] Surface wiring reviewed where applicable.
- [x] Scaffold, dead-code, and fallback signals classified.
- [x] Security and reliability risks reviewed.

## Findings

- Binary keeps the existing provider fetch and worker event-spine evidence path
  through `tdw-service-api`.
- `--contract` exposes the worker queue contract and `--durable-smoke`
  exercises the SQLite durable scheduler with enqueue, lease, complete, and
  stats.
- The library owns `WorkerJob`, `WorkerLease`, `DeadLetterRecord`,
  `WorkerQueueStats`, an in-memory contract backend, and
  `SqliteWorkerQueue`.
- `PgWorkerQueue` is available behind `--features postgres` and mirrors the
  SQLite durable scheduler contract against `system.worker_jobs` with
  `FOR UPDATE SKIP LOCKED` leasing for distributed workers.
- Durable scheduler coverage includes priority leasing, not-before scheduling,
  reconnect persistence, expired-lease reaping, retry counters,
  dead-lettering, idempotent enqueue, and idempotent completion.
- Postgres migration catalog includes `20260521_0008_worker_queue.sql` for the
  distributed worker queue table and indexes. Live Postgres verification is
  double-gated by `--features postgres` and `TDW_POSTGRES_TEST_URL`.
- Scan signal is the worker sample call; no stub, copied FinX-XR, OpenBB, or
  fallback path was found.

## Verification

- Focused G007 command passed: `cargo test -p tdw-acp -p tdw-app-client -p tdw-app-server -p tdw-exec -p tdw-runtime -p tdw-service-api -p tdw-tui -p tdw-cli -p tdw-mcp -p tdw-service -p tdw-worker`.
- Worker durable scheduler checks passed: `cargo test -p tdw-worker`,
  `cargo clippy -p tdw-worker --all-targets -- -D warnings`, and
  `cargo run -p tdw-worker -- --durable-smoke`.
- G004 production backend checks: `cargo check -p tdw-worker --features
  postgres`, `cargo test -p tdw-worker --features postgres`, and
  `cargo test -p tdw-migration`.

## Verdict

Ready with follow-ups. The worker now has both an embedded SQLite durable
scheduler for local and single-process deployments and a feature-gated
Postgres distributed scheduler for multi-worker deployments. Remaining
follow-ups are operational rollout decisions rather than missing scheduler
contract coverage.
