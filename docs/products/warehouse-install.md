# Self-hosted warehouse: install and 15-minute evaluation

Audience: a quant/platform engineer evaluating FinX-Plattform as a self-hosted,
event-sourced market-data warehouse. This page composes the operator runbooks
into one customer path — install, prove it works, know where everything lives.

Time budget: ~15 minutes on a machine with Docker (most of it image builds on
the first run).

## 1. Prerequisites (2 min)

- Docker Engine + Docker Compose v2 (`docker compose version` ≥ 2.20)
- ~10 GB free disk for images + named volumes
- Free host ports: `5432` (Postgres), `8123`/`9000` (ClickHouse), `6333`
  (Qdrant), `7700` (Meilisearch), `9001`/`9002` (MinIO), `6379` (Redis)

```bash
git clone https://github.com/xrey167/FinX-Plattform.git
cd FinX-Plattform
cp .env.example .env
# Non-loopback MCP binds must be authenticated:
echo "TDW_MCP_HTTP_TOKEN=$(openssl rand -hex 24)" >> .env
docker compose --profile full config >/dev/null && echo "compose model OK"
```

The `.env` defaults work for a local evaluation; every `TDW_*` variable is
documented inline in [`.env.example`](../../.env.example).

## 2. Bring up the durable backend (3 min)

```bash
docker compose --profile live up -d --build
docker compose --profile live logs tdw-bootstrap
```

`tdw-bootstrap` is one-shot and idempotent: it creates the Postgres schemas and
the S3 bucket, writes a marker object, and exits 0. Re-running it is safe.
Details: [data backend runbook](../release/data-backend-runbook.md).

**Checkpoint A:** the bootstrap log ends with the success line and
`docker compose --profile live ps` shows postgres + minio healthy.

## 3. Bring up the full stack (5 min)

```bash
docker compose --profile full up -d --build
docker compose --profile full ps
```

This adds ClickHouse, Qdrant, Meilisearch, Redis, and the two long-running
binaries: `tdw-service` (the daemon) and `tdw-worker` (the job runner). Both
run with `TDW_PROFILE=docker`. Per-service health and the full smoke
choreography: [local stack runbook](../release/local-stack-runbook.md).

**Checkpoint B:** every service in `ps` is `running` (or `healthy` where a
healthcheck is defined).

## 4. Prove the spine end-to-end (3 min)

Run the packaged smokes — the same ones CI gates every merge on:

```bash
docker compose --profile full run --rm tdw-service --smoke AAPL
docker compose --profile full run --rm tdw-worker --durable-smoke
docker compose --profile tools run --rm tdw-mcp --streamable-http-smoke
```

**Checkpoint C:** each prints its success JSON/line and exits 0. You have now
exercised: provider fetch → event spine → storage write/read round-trip,
durable worker queue against Postgres, and an MCP tool-call round-trip.

## 5. Query your data (2 min)

```bash
docker compose --profile live exec -T postgres psql -U tdw -d tdw \
  -c "select count(*) from tdw_rollout;"
docker compose --profile full exec -T clickhouse \
  clickhouse-client --query "show tables"
```

State lives in named Docker volumes (`postgres-data`, `clickhouse-data`,
`qdrant-data`, `meili-data`, `minio-data`) — survives `compose down`, removed
only by `compose down -v`.

## Where to go next

| Need | Doc |
|---|---|
| Production ingress auth (OIDC, fail-closed) | [production-auth-oidc](../release/production-auth-oidc.md) |
| Worker scale-out / deployment shapes | [worker-deployment](../release/worker-deployment.md) |
| Remote MCP for your agents | [mcp-remote-deployment](../release/mcp-remote-deployment.md) + [quickstart](mcp-quickstart.md) |
| Manual deep verification checklist | [live-stack-smoke](../release/live-stack-smoke.md) |
| Upgrades | pull the new tag, `docker compose --profile full up -d --build`; `tdw-bootstrap` re-runs idempotently. Release notes: [CHANGELOG](../../CHANGELOG.md) |

Known evaluation caveats: this 15-minute path runs the offline `docker`
profile, keeping the evaluation deterministic and free of API-key
requirements. Live provider calls are verified separately (nightly
`live-smoke` CI job); a continuous Binance→ClickHouse streaming soak is on
the roadmap (go-live P2.4) and not yet part of CI.
