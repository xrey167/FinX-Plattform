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

| Service             | Image                              | Notes                                                                       |
|---------------------|------------------------------------|-----------------------------------------------------------------------------|
| `postgres`          | `postgres:17-alpine`               | Healthcheck via `pg_isready`                                                |
| `clickhouse`        | `clickhouse/clickhouse-server:25.5`| OLAP backend                                                                |
| `qdrant`            | `qdrant/qdrant:latest`             | Vector backend                                                              |
| `meilisearch`       | `getmeili/meilisearch:latest`      | Lexical backend                                                             |
| `minio`             | `minio/minio:latest`               | Healthcheck via `/minio/health/live`                                        |
| `minio-init`        | `minio/mc:latest`                  | Creates the `tdw-default` bucket once, then exits                           |
| `tdw-bootstrap`     | built from `Dockerfile.bootstrap`  | Applies G013 Postgres schemas, writes the S3 marker, and creates the ClickHouse `tdw` DB + marker table, Qdrant `tdw-default` collection, and Meilisearch `tdw-default` index, then exits |
| `tdw-worker-serve`  | `docker/tdw-worker.Dockerfile`     | Long-running `tdw-worker --serve` lease loop (SQLite-backed); starts after bootstrap succeeds |

`tdw-bootstrap` runs after `postgres` is healthy, `minio-init` has finished,
and ClickHouse/Qdrant/Meilisearch have started. It emits one structured JSON
line per step on stdout, and is idempotent (all creates use
`IF NOT EXISTS`/exists-checks). `tdw-worker-serve` then starts as the first
long-running application service and stays up (`restart: unless-stopped`).

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

Check the ClickHouse baseline:

```powershell
docker compose exec clickhouse clickhouse-client -u tdw --password tdw --query "exists table tdw._tdw_bootstrap_marker"
```

Returns `1`. Check the Qdrant collection and Meilisearch index:

```powershell
curl -s http://localhost:6333/collections/tdw-default | findstr status
curl -s http://localhost:7700/indexes/tdw-default | findstr uid
```

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
| `tdw-bootstrap` exits 7                                | ClickHouse unreachable or DDL failed. Check the `clickhouse` service and the logged statement.     |
| `tdw-bootstrap` exits 8                                | Qdrant unreachable or collection create failed. Check the `qdrant` service.                        |
| `tdw-bootstrap` exits 9                                | Meilisearch unreachable or index create task failed. Check the `meilisearch` service.              |
| `port is already allocated`                            | Another process is using `5432`, `9001`, or `9002`. Stop it or change the port mapping in compose. |
| `permission denied while connecting to the Docker daemon` | Run with `sudo` on Linux, or add your user to the `docker` group.                                  |

## What this does NOT do

- Start the full application surface. The `live` profile now starts one
  long-running service (`tdw-worker-serve`); `tdw-service` and `tdw-mcp`
  long-running modes remain follow-ups.
- Back the live worker with Postgres. `tdw-worker --serve` currently uses the
  SQLite durable queue (on the `worker-data` volume); a Postgres-backed
  `--serve` is a tracked follow-up.
- Define rich domain schemas. The ClickHouse table, Qdrant collection, and
  Meilisearch index created here are baseline markers proving the backends are
  reachable and writable; application tables/collections are still created on
  first domain write.
- Configure TLS, secrets management, or non-root containers. That is a
  hardening pass tracked in G014's follow-up slices.

## See also

- `docs/quality/production-storage-transports.md` - per-backend
  recipes and the full G010 status table.
- `docs/quality/production-transport-status.md` - workspace-wide
  transport status across G010-G014.
- `crates/tdw-bootstrap/src/main.rs` - exit-code legend and
  per-step JSON shape.
