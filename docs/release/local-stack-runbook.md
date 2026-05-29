# Local stack runbook - the full deployed stack end to end

One-page guide for standing up the **entire** FinX-Plattform stack on one
machine - every storage backend plus the four application binaries
(`tdw-service`, `tdw-worker`, `tdw-mcp`, `tdw-cli`) - and proving it runs.

This is the application-services slice the
[`data-backend-runbook`](./data-backend-runbook.md) deferred ("What this does
NOT do: start any application services"). The backend runbook brings up
Postgres + S3 + the durable schemas; this runbook adds the rest of the
backends and the binaries on top.

Providers and LLM/embedding transports run **offline** here (fixture/cassette,
`TDW_PROFILE=docker`) - no external API keys are required to exercise the stack.

## Compose profiles

`docker-compose.yaml` (project `finx-plattform`) groups services into profiles
so you bring up only what you need:

| Profile   | Services                                                                 | Use for                                            |
|-----------|--------------------------------------------------------------------------|----------------------------------------------------|
| `minimal` | postgres, clickhouse                                                     | Core relational + columnar backends only           |
| `full`    | postgres, clickhouse, qdrant, meilisearch, minio (S3), redis, `tdw-service`, `tdw-worker` | The full deployed stack: all backends + long-running binaries |
| `live`    | postgres, minio, minio-init, `tdw-bootstrap`                             | Durable-persistence schema bootstrap (see backend runbook) |
| `tools`   | `tdw-mcp`, `tdw-cli`                                                      | One-shot tool binaries (MCP server, CLI)           |

Profiles compose: `--profile full --profile tools` brings up backends,
long-running services, and the tool binaries together.

## Prerequisites

- Docker Engine + Docker Compose v2 (`docker compose version`, 2.20+).
- `git clone` of this repo; working directory at the repo root.
- Free host ports: `5432` (postgres), `8123`/`9000` (clickhouse),
  `6333`/`6334` (qdrant), `7700` (meilisearch), `9001`/`9002` (minio),
  `6379` (redis).

## Bring up the full stack

```powershell
docker compose --profile full --profile tools build
docker compose --profile full up -d
```

`tdw-service` and `tdw-worker` start once every backend they `depends_on` is
healthy/started. Both carry `TDW_PROFILE=docker` (offline providers).

Check everything is up:

```powershell
docker compose --profile full ps
```

## Verify each binary

`tdw-service` runs a one-shot historical smoke (`--smoke AAPL`) against the
live backends and exits 0 on success:

```powershell
docker compose --profile full logs tdw-service
```

`tdw-worker` is the supervised lease loop (see
[`worker-deployment`](./worker-deployment.md)); confirm it claims its lease and
idles without error:

```powershell
docker compose --profile full logs tdw-worker
```

Run the CLI tool against the stack:

```powershell
docker compose --profile tools run --rm tdw-cli AAPL
```

Exercise the MCP server (stdio); for the HTTP transport behind a proxy see
[`mcp-remote-deployment`](./mcp-remote-deployment.md):

```powershell
docker compose --profile tools run --rm tdw-mcp
```

## Bootstrap durable schemas (optional, for stateful runs)

The `full` profile starts the backends but does not apply the durable Postgres
schemas. To make the stack stateful, run the `live` bootstrap once (it is
idempotent), per the [`data-backend-runbook`](./data-backend-runbook.md):

```powershell
docker compose --profile live up -d --build
docker compose logs tdw-bootstrap   # expect {"step":"done","status":"ok",...}
```

## Tear down

```powershell
docker compose --profile full down        # keep volumes
docker compose --profile full down -v      # drop all data
```

## CI evidence - the stack is proven on every push

Local Docker is not required to trust that the stack runs. The pipeline builds
and exercises it continuously:

| Evidence                                   | Workflow / job                                  | What it proves                                                       |
|--------------------------------------------|-------------------------------------------------|----------------------------------------------------------------------|
| `Container Image (tdw-{service,cli,mcp,worker})` | `.github/workflows/ci.yml` (every PR + main)    | All four images build from `docker/*.Dockerfile` and pass a Trivy scan |
| `Integration, Property, and E2E Subset`    | `.github/workflows/ci.yml` (every PR + main)    | The integration/e2e subset runs against service-backed paths         |
| `e2e-full`                                 | `.github/workflows/nightly.yml`                 | `docker compose --profile full config` validates, then `cargo test --workspace --features e2e` |

A green run on these jobs is the deployed-stack acceptance signal; this runbook
is the manual equivalent for a local box.

## What this does NOT do

- Configure TLS, secrets management, or non-root containers - that is a
  hardening pass, not covered here. For the MCP HTTP transport behind a
  TLS/OAuth proxy see [`mcp-remote-deployment`](./mcp-remote-deployment.md).
- Connect to live external provider/LLM APIs - the stack runs offline
  (`TDW_PROFILE=docker`). Supply keys in `.env` only if you want live data.
- Scale beyond a single host (no orchestration, replicas, or external load
  balancing).

## See also

- [`data-backend-runbook`](./data-backend-runbook.md) - Postgres + S3 + durable
  schema bootstrap (the `live` profile).
- [`worker-deployment`](./worker-deployment.md) - `tdw-worker` PgWorkerQueue
  rollout, supervision, and monitoring.
- [`mcp-remote-deployment`](./mcp-remote-deployment.md) - MCP Streamable HTTP
  behind a TLS/OAuth proxy.
- `docs/quality/production-transport-status.md` - workspace-wide transport
  status across G010-G014.
