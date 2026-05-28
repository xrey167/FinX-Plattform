# Data backend runbook - `docker compose --profile live`

One-page operational guide for bringing the FinX-Plattform data
backend (Postgres + S3/MinIO + the durable-persistence schemas)
live on a fresh machine.

## Prerequisites

- Docker Engine + Docker Compose v2 (`docker compose version` 2.20 or newer).
- `git clone` of this repo, with working directory at the repo root.
- No other process bound to host ports `5432`, `9001`, or `9002`.

## One-time setup

```powershell
copy .env.example .env
```

Edit `.env` if you want to change credentials or supply LLM /
provider keys. The defaults work as-is for local development.

## Bring the backend live

```powershell
docker compose --profile live up -d --build
```

This starts:

| Service           | Image                          | Notes                                                      |
|-------------------|--------------------------------|------------------------------------------------------------|
| `postgres`        | `postgres:17-alpine`           | Healthcheck via `pg_isready`                               |
| `minio`           | `minio/minio:latest`           | Healthcheck via `/minio/health/live`                       |
| `minio-init`      | `minio/mc:latest`              | Creates the `tdw-default` bucket once, then exits          |
| `tdw-bootstrap`   | built from `Dockerfile.bootstrap` | Applies all G013 Postgres schemas and writes the S3 marker, then exits |

`tdw-bootstrap` will only run after `postgres` is healthy and
`minio-init` has finished. It emits one structured JSON line per
step on stdout.

## Verify the bootstrap

Tail the bootstrap logs:

```powershell
docker compose logs tdw-bootstrap
```

Expected last line:

```
{"step":"done","status":"ok","detail":"data backend live"}
```

Check Postgres tables:

```powershell
docker compose exec postgres psql -U tdw -d tdw -c "\dt"
```

You should see:

- `tdw_outbox`
- `tdw_snapshot`
- `tdw_bus`
- `tdw_sessions`
- `tdw_sessions_permission_state`
- `tdw_sessions_pending_approvals`
- `tdw_sessions_cost_ledger`

Check the MinIO bucket + marker object:

```powershell
docker compose exec minio sh -c "mc alias set local http://localhost:9000 minio minio123 && mc ls local/tdw-default"
```

You should see `_tdw_bootstrap_marker`.

## Re-running bootstrap

Bootstrap is idempotent (all `CREATE TABLE` statements use
`IF NOT EXISTS`; the S3 marker is overwritten). To re-run after a
config change or to refresh schemas:

```powershell
docker compose run --rm tdw-bootstrap
```

## Tear down

Stop services but keep volumes:

```powershell
docker compose --profile live down
```

Drop volumes (loses all data):

```powershell
docker compose --profile live down -v
```

## Troubleshooting

| Symptom                                                | Fix                                                                                                |
|--------------------------------------------------------|----------------------------------------------------------------------------------------------------|
| `tdw-bootstrap` exits 2                                | Required env var missing in compose. Check `environment:` block on the service.                    |
| `tdw-bootstrap` exits 3                                | Cannot reach Postgres. Wait for `postgres` healthcheck, then re-run bootstrap.                     |
| `tdw-bootstrap` exits 4                                | Postgres reachable but DDL failed. `docker compose logs tdw-bootstrap` shows the failing schema.   |
| `tdw-bootstrap` exits 5                                | MinIO bucket missing. `minio-init` failed; re-run with `docker compose up minio-init`.             |
| `port is already allocated`                            | Another process is using `5432`, `9001`, or `9002`. Stop it or change the port mapping in compose. |
| `permission denied while connecting to the Docker daemon` | Run with `sudo` on Linux, or add your user to the `docker` group.                                  |

## What this does NOT do

- Start any application services (`tdw-service`, `tdw-worker`,
  `tdw-mcp`). Those are separate slices in a future PR.
- Bootstrap ClickHouse / Qdrant / Meilisearch schemas. They are
  brought up by the `full` compose profile but their schemas are
  application-defined when the first write arrives. Re-run
  bootstrap once those use cases land.
- Configure TLS, secrets management, or non-root containers.
  That is a hardening pass tracked in G014's follow-up slices.

## See also

- `docs/quality/production-storage-transports.md` - per-backend
  recipes and the full G010 status table.
- `docs/quality/production-transport-status.md` - workspace-wide
  transport status across G010-G014.
- `crates/tdw-bootstrap/src/main.rs` - exit-code legend and
  per-step JSON shape.
