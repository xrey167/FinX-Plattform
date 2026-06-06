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
| `tdw-worker-serve`  | `docker/tdw-worker.Dockerfile` (`FEATURES=postgres`) | Long-running `tdw-worker --serve` over **Postgres** (`PgWorkerQueue`, self-migrates `system.worker_jobs`); starts after bootstrap |
| `tdw-service-daemon`| `docker/tdw-service.Dockerfile`    | Long-running daemon, binds `0.0.0.0:7878` (`TDW_DAEMON_TCP_BIND`); **internal-only** (not host-published — its transport is unauthenticated plaintext) |
| `tdw-mcp-serve`     | `docker/tdw-mcp.Dockerfile`        | Long-running MCP Streamable HTTP at `0.0.0.0:8788` (host `8788`); daemon tools routed to `tdw-service-daemon:7878`; **requires** `TDW_MCP_HTTP_TOKEN` |

`tdw-bootstrap` runs after `postgres` is healthy, `minio-init` has finished,
and ClickHouse/Qdrant/Meilisearch have started. It emits one structured JSON
line per step on stdout, and is idempotent (all creates use
`IF NOT EXISTS`/exists-checks). The long-running application services then start
and stay up (`restart: unless-stopped`): the Postgres-backed worker, the daemon
(`tdw-service-daemon`), and the MCP HTTP server (`tdw-mcp-serve`).

`TDW_MCP_HTTP_TOKEN` is **required** (no default): `docker compose --profile
live up` fails fast if it is unset. Set a strong, unique value in `.env`
(e.g. `openssl rand -hex 32`) — the MCP server refuses a non-loopback bind
without it. Front it with a TLS/OAuth reverse proxy per
[`mcp-remote-deployment.md`](mcp-remote-deployment.md) for any non-local use.

### Daemon TCP transport — bind safely

The daemon's TCP transport (`TDW_DAEMON_TCP_BIND`) **defaults to loopback**
(`127.0.0.1:7878`) when the variable is unset, so an out-of-the-box run is not
reachable off-host. Binding a routable address is an **explicit opt-in**.

- **Safe default**: leave `TDW_DAEMON_TCP_BIND` unset (or set `127.0.0.1:7878`).
  Anything that needs the daemon (the MCP server, workers) reaches it over the
  loopback/compose network.
- **Container / cross-host**: the `live` compose stack sets
  `TDW_DAEMON_TCP_BIND=0.0.0.0:7878` so sibling containers can reach it, but the
  port is **internal-only** (not host-published) and the compose network is the
  trust boundary. Do **not** publish `7878` to the host or a routable interface.
- **Prominent warning**: when the daemon binds a **non-loopback** address with
  **no auth-backed policy attached**, it logs a `SECURITY WARNING` to stderr at
  startup (the daemon's transport is unauthenticated plaintext — any host that
  can reach the port can drive it).

If you must expose the daemon beyond loopback, protect it with one of:

- **Ingress auth policy**: configure `TDW_OIDC_*` (see
  [`production-auth-oidc.md`](production-auth-oidc.md)) so dispatches require a
  verified token; cryptographic JWT verification (RS256/ES256 signature + claim
  checks) is enforced by `tdw-auth-oidc`.
- **mTLS**: terminate mutual-TLS at a sidecar/proxy in front of `7878` so only
  clients presenting a trusted client certificate reach the daemon.
- **Reverse proxy**: front the daemon with a TLS/OAuth reverse proxy (as for the
  MCP server) and keep the daemon itself bound to loopback inside the trust
  boundary.

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

- Auto-configure daemon auth. This compose setup does not wire daemon auth, so
  `tdw-service-daemon` boots fail-closed (no policy attached) and dispatched
  operations return `Failed`; daemon-backed MCP tool calls reach the daemon but
  get that `Failed` result. Deterministic offline MCP tools work regardless. A
  `prod`/`production` daemon attaches an auth-backed policy when `TDW_OIDC_*` is
  configured — see [`production-auth-oidc.md`](./production-auth-oidc.md).
- Back the daemon's own stores with Postgres. `tdw-service-daemon` uses in-memory
  session/rollout defaults for boot; wiring its stores to the live Postgres is a
  further enhancement (the worker IS Postgres-backed).
- Define rich domain schemas. The ClickHouse table, Qdrant collection, and
  Meilisearch index created here are baseline markers proving the backends are
  reachable and writable; application tables/collections are still created on
  first domain write.
- Configure TLS, secrets management, or non-root containers. That is a
  hardening pass tracked in G014's follow-up slices.

## See also

- `docs/release/secrets-and-tls.md` - systemd/Kubernetes secret injection, TLS, and `TDW_MCP_HTTP_TOKEN` rotation.
- `docs/quality/production-storage-transports.md` - per-backend
  recipes and the full G010 status table.
- `docs/quality/production-transport-status.md` - workspace-wide
  transport status across G010-G014.
- `crates/tdw-bootstrap/src/main.rs` - exit-code legend and
  per-step JSON shape.
