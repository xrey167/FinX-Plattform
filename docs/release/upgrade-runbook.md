# Upgrade runbook — TDW images, schema migrations, and infra images

Operational guide for upgrading the FinX-Plattform `live` stack: bumping TDW
application image tags, applying Postgres and ClickHouse schema migrations, and
upgrading the infrastructure images (Postgres, ClickHouse, Qdrant, Meilisearch,
MinIO).

**Always take a backup before any upgrade.** See
[`backup-restore-runbook.md`](backup-restore-runbook.md) for the full procedure.

---

## Overview: upgrade order

1. **Backup** — full cold backup per [`backup-restore-runbook.md`](backup-restore-runbook.md).
2. **Pull / rebuild** the new TDW images (or update pinned infra tags).
3. **Apply schema migrations** — Postgres and ClickHouse DDL is applied
   automatically by `tdw-bootstrap` on startup (idempotent `CREATE … IF NOT
   EXISTS`). See [Schema migrations](#schema-migrations) for the full mechanism.
4. **Bring the stack up** — `docker compose --profile live up -d`.
5. **Verify health** — ops endpoints per [`service-operability.md`](service-operability.md).
6. **Run smoke checklist** — [`live-stack-smoke.md`](live-stack-smoke.md).
7. **Rollback** if any step fails — previous image tag, or restore from backup.

---

## Upgrading TDW application images

The `live` profile builds four TDW images locally from the repo:

| Compose service | Dockerfile | Image tag |
|---|---|---|
| `tdw-service-daemon` | `docker/tdw-service.Dockerfile` | `finx-plattform/tdw-service:local-pg` |
| `tdw-worker-serve` | `docker/tdw-worker.Dockerfile` | `finx-plattform/tdw-worker:local-pg` |
| `tdw-mcp-serve` | `docker/tdw-mcp.Dockerfile` | `finx-plattform/tdw-mcp:local` |
| `tdw-bootstrap` | `Dockerfile.bootstrap` | (built inline) |

### Rolling to a new release tag / SHA

1. Check out the target commit or tag:

   ```powershell
   git fetch origin
   git checkout v1.3.0   # or a specific SHA
   ```

2. Rebuild all images against the new source:

   ```powershell
   docker compose --profile live build --no-cache
   ```

   `--no-cache` ensures no stale layer is reused. Omit it if you are confident
   the base image layers are unchanged.

3. Bring the stack up with the new images:

   ```powershell
   docker compose --profile live up -d
   ```

   Compose replaces only the containers whose image changed. The durable volumes
   (`postgres-data`, `clickhouse-data`, etc.) are untouched.

4. Tail the bootstrap log to confirm schema migration completed successfully:

   ```powershell
   docker compose logs tdw-bootstrap
   # Expect: {"step":"done","status":"ok","detail":"data backend live"}
   ```

5. Verify service health (see [Verify health endpoints](#verify-health-endpoints)).

### Rollback to the previous tag

If the new build is broken:

```powershell
# Check out the previous commit or tag.
git checkout v1.2.0

# Rebuild and restart.
docker compose --profile live build --no-cache
docker compose --profile live up -d
```

Because volumes are not touched by a rollback, no data restore is needed
unless the new migration was destructive (which is not possible in this repo —
see [Schema migrations: no destructive migrations](#no-destructive-migrations)
below).

---

## Schema migrations

### Mechanism

This repo uses a **library-driven, `CREATE … IF NOT EXISTS` migration catalog**
embedded in the `tdw-migration` crate
(`crates/tdw-migration/src/lib.rs`). There is no external migration runner
(no `sqlx migrate`, no `flyway`).

**How migrations are applied:**

- **At compose startup**, the `tdw-bootstrap` one-shot service
  (`crates/tdw-bootstrap/src/main.rs`) calls each G013 durable-persistence
  crate's `ensure_schema()` method. These calls issue `CREATE TABLE IF NOT
  EXISTS` DDL directly against Postgres via `sqlx`. The `tdw_outbox`,
  `tdw_snapshot`, `tdw_bus`, `tdw_sessions*`, and `tdw_rollout` tables are all
  created this way. Similarly, ClickHouse, Qdrant, and Meilisearch baseline
  schemas are created by bootstrap.

- **At service connect time**, the `tdw-worker-serve` service calls
  `PgWorkerQueue::connect(url).migrate()` on startup, which creates
  `system.worker_jobs` and its ready-index with `IF NOT EXISTS`. The daemon's
  session and rollout stores (`PgSessionStore`, `PgRollout`) similarly call
  `ensure_schema()` on connect.

- **The `tdw-migration` crate** (`crates/tdw-migration/src/lib.rs`) is a
  catalog of static SQL files (`migrations/postgres/*.sql` and
  `migrations/clickhouse/*.sql`) with version strings like `20260521_0001`.
  This catalog is consumed by the `xtask` offline tool and can be inspected for
  planning, but it is **not** what runs at service startup. The runtime path is
  the `ensure_schema()` / `PgWorkerQueue::migrate()` calls described above.

**Current migration counts** (as of the catalog in this tree):

```powershell
# Inspect offline — prints postgres=N clickhouse=N
cargo run -p xtask --target-dir target -- migrate status
```

**Inspect the planned apply order offline:**

```powershell
cargo run -p xtask --target-dir target -- migrate up
# Prints each migration version + name in apply order, no DB connection required.
```

**Note:** `xtask migrate up` is a **dry-run planning tool only** — it prints
what would be applied but does not connect to any database. The actual DDL is
applied by `tdw-bootstrap` and the service startup paths described above.

### Migration catalog: Postgres

SQL files are embedded at compile time via `include_str!` in
`crates/tdw-migration/src/lib.rs`. All migrations are `CREATE … IF NOT EXISTS`
and are therefore safe to re-run.

Current Postgres migrations in version order:

| Version | Name | File |
|---|---|---|
| `20260521_0001` | `init_schemas` | `migrations/postgres/20260521_0001_init_schemas.sql` |
| `20260521_0002` | `bronze_market_data` | `migrations/postgres/20260521_0002_bronze_market_data.sql` |
| `20260521_0003` | `agents_and_evals` | `migrations/postgres/20260521_0003_agents_and_evals.sql` |
| `20260521_0004` | `agent_runtime` | `migrations/postgres/20260521_0004_agent_runtime.sql` |
| `20260521_0005` | `event_spine` | `migrations/postgres/20260521_0005_event_spine.sql` |
| `20260521_0006` | `parity_layer` | `migrations/postgres/20260521_0006_parity_layer.sql` |
| `20260521_0007` | `kg_tags_feature_store` | `migrations/postgres/20260521_0007_kg_tags_feature_store.sql` |
| `20260521_0008` | `worker_queue` | `migrations/postgres/20260521_0008_worker_queue.sql` |
| `20260528_0001` | `reference_master` | `migrations/postgres/20260528_0001_reference_master.sql` |
| `20260528_0002` | `symbol_history` | `migrations/postgres/20260528_0002_symbol_history.sql` |
| `20260528_0003` | `trading_calendar` | `migrations/postgres/20260528_0003_trading_calendar.sql` |
| `20260607_0001` | `price_alerts` | `migrations/postgres/20260607_0001_price_alerts.sql` |
| `20260607_0002` | `function_steps` | `migrations/postgres/20260607_0002_function_steps.sql` |
| `20260607_0003` | `identity_users` | `migrations/postgres/20260607_0003_identity_users.sql` |
| `20260608_0001` | `identity_sessions` | `migrations/postgres/20260608_0001_identity_sessions.sql` |
| `20260608_0002` | `identity_reset_tokens` | `migrations/postgres/20260608_0002_identity_reset_tokens.sql` |

### Migration catalog: ClickHouse

| Version | Name | File |
|---|---|---|
| `20260521_0001` | `init_databases` | `migrations/clickhouse/20260521_0001_init_databases.sql` |
| `20260521_0002` | `bronze_ohlcv` | `migrations/clickhouse/20260521_0002_bronze_ohlcv.sql` |
| `20260528_0001`–`0011` | raw + analytics | `migrations/clickhouse/20260528_000N_*.sql` |
| `20260528_0012`–`0020` | analytics indicators/UDFs | `migrations/clickhouse/20260528_00NN_*.sql` |

### No destructive migrations

`xtask migrate down` prints a notice that no destructive migration is run:

> `offline migrate down plan: no destructive migration is run by xtask scaffold`

All catalog migrations are additive (`CREATE … IF NOT EXISTS`). No `DROP`,
`ALTER … DROP COLUMN`, or `TRUNCATE` statements exist in the migration catalog.
Rollback is therefore always safe at the DDL level: a previous image will find
its expected tables still present.

### Applying migrations manually (troubleshooting)

If `tdw-bootstrap` exits non-zero and schema creation failed, you can apply
individual migration SQL files directly:

```powershell
# Postgres — apply a single migration file
docker compose exec -T postgres `
  psql -U tdw -d tdw `
  -f /dev/stdin < migrations/postgres/20260607_0001_price_alerts.sql

# Or exec into the container:
docker compose cp migrations/postgres/20260607_0001_price_alerts.sql postgres:/tmp/
docker compose exec postgres psql -U tdw -d tdw -f /tmp/20260607_0001_price_alerts.sql
```

```powershell
# ClickHouse — apply a single migration file
docker compose cp migrations/clickhouse/20260528_0001_raw_equity_historical.sql clickhouse:/tmp/
docker compose exec -T clickhouse `
  clickhouse-client --user tdw --password tdw `
  --multiquery --queries-file /tmp/20260528_0001_raw_equity_historical.sql
```

After manual application, re-run bootstrap to confirm all steps pass:

```powershell
docker compose run --rm tdw-bootstrap
```

---

## Upgrading infrastructure images

The `live` profile pins these infra image tags in `docker-compose.yaml`:

| Service | Image | Pin |
|---|---|---|
| `postgres` | `postgres:17-alpine` | major.minor pinned |
| `clickhouse` | `clickhouse/clickhouse-server:25.5` | major.minor pinned |
| `qdrant` | `qdrant/qdrant:latest` | floating |
| `meilisearch` | `getmeili/meilisearch:latest` | floating |
| `minio` | `minio/minio:latest` | floating |

For floating-tag services (`qdrant`, `meilisearch`, `minio`) pull the latest
image before each upgrade cycle to get the most recent patch:

```powershell
docker compose --profile live pull qdrant meilisearch minio
```

### Postgres minor version upgrade (e.g. 17.x → 17.y)

Minor-version upgrades within the same major are in-place compatible. No data
migration is needed.

1. Update the image tag in `docker-compose.yaml` to `postgres:17.y-alpine`.
2. Bring the stack down and up: `docker compose --profile live up -d`.
3. Verify: `docker compose exec postgres psql -U tdw -d tdw -c "SELECT version();"`.

### Postgres major version upgrade (e.g. 17 → 18)

Major-version upgrades require a logical dump/restore because the on-disk data
format changes between major versions.

1. **Back up** with `pg_dump` (see [backup-restore-runbook.md](backup-restore-runbook.md)).
2. Update the image tag in `docker-compose.yaml`.
3. Remove the old data volume: `docker volume rm finx-plattform_postgres-data`.
4. Start the new Postgres: `docker compose up -d postgres`.
5. Restore from the logical dump: `pg_restore -U tdw -d tdw /tmp/tdw.dump`.
6. Run `docker compose run --rm tdw-bootstrap` to re-apply bootstrap schemas.

Upstream reference: https://www.postgresql.org/docs/current/upgrading.html

### ClickHouse minor version upgrade (25.x → 25.y)

ClickHouse minor releases are generally backward-compatible for the data format.

1. Update the image tag in `docker-compose.yaml` to
   `clickhouse/clickhouse-server:25.y`.
2. `docker compose --profile live up -d clickhouse`.
3. Verify: `docker compose exec clickhouse clickhouse-client -u tdw --password tdw --query "SELECT version()"`.

### ClickHouse major version upgrade

ClickHouse may introduce data format or dictionary/MV incompatibilities across
major versions. Always:

1. Back up the `clickhouse-data` volume (cold tarball + `BACKUP DATABASE` zip).
2. Review the upstream changelog for breaking changes:
   https://clickhouse.com/docs/en/whats-new/changelog
3. Follow the same minor-upgrade steps; if the container fails to start, restore
   from backup.

### Qdrant version upgrade

Qdrant may change its on-disk storage format between versions, especially for
collection segments. Always:

1. Take a Qdrant snapshot of all collections before upgrading (see
   [backup-restore-runbook.md](backup-restore-runbook.md)).
2. Update the tag or pull `latest`.
3. If the new version cannot read the existing segments, delete the
   `qdrant-data` volume and restore from the snapshot.

Upstream data format migration notes:
https://qdrant.tech/documentation/guides/migration/

### Meilisearch version upgrade

Meilisearch's on-disk index format may be incompatible between minor versions.

1. Export a dump before upgrading (see [backup-restore-runbook.md](backup-restore-runbook.md)).
2. Pull the new image.
3. If the new version cannot open the existing index data, recreate the volume
   and import from the dump using `--import-dump` (see Meilisearch restore
   procedure in the backup runbook).

Upstream version compatibility notes:
https://www.meilisearch.com/docs/learn/update_and_migration/updating

### MinIO version upgrade

MinIO stores objects as files and is backward-compatible for standard S3
operations across versions. Update the tag, restart, and verify bucket access:

```powershell
docker compose pull minio
docker compose --profile live up -d minio
docker compose exec minio sh -c "mc alias set local http://localhost:9000 minio minio123 && mc ls local/tdw-default"
```

---

## Verify health endpoints

After any upgrade, confirm all three long-running services are healthy before
declaring success. Full endpoint reference: [`service-operability.md`](service-operability.md).

```powershell
# Daemon liveness (port 9102, TDW_DAEMON_HTTP_BIND)
curl -fsS http://localhost:9102/health && Write-Host "daemon: OK"

# MCP liveness (port 9103, TDW_MCP_OPS_BIND)
curl -fsS http://localhost:9103/health && Write-Host "mcp: OK"

# Worker liveness (port 9101, TDW_WORKER_HTTP_BIND)
curl -fsS http://localhost:9101/health && Write-Host "worker: OK"

# Readiness (checks downstream stores are reachable)
curl -fsS http://localhost:9102/ready && Write-Host "daemon ready"
curl -fsS http://localhost:9103/ready && Write-Host "mcp ready"
curl -fsS http://localhost:9101/ready && Write-Host "worker ready"
```

A `200` from `/health` means the process is serving. A `200` from `/ready`
means the downstream dependencies (Postgres, daemon) are reachable. A `503`
from `/ready` means the service is alive but a dependency is down.

Then run the full manual smoke checklist: [`live-stack-smoke.md`](live-stack-smoke.md).

---

## Upgrade checklist

- [ ] Cold backup taken per [`backup-restore-runbook.md`](backup-restore-runbook.md).
- [ ] Git SHA / tag of the new release recorded alongside the backup.
- [ ] TDW images rebuilt with `docker compose --profile live build --no-cache`.
- [ ] `tdw-bootstrap` logs confirm `{"step":"done","status":"ok","detail":"data backend live"}`.
- [ ] All three `/health` endpoints return `200`.
- [ ] All three `/ready` endpoints return `200`.
- [ ] Smoke checklist ([`live-stack-smoke.md`](live-stack-smoke.md)) passes.
- [ ] Previous image tag noted for rollback (git checkout + rebuild).

---

## See also

- [`backup-restore-runbook.md`](backup-restore-runbook.md) — full backup and
  restore procedures for all five named volumes.
- [`service-operability.md`](service-operability.md) — daemon, MCP, and worker
  health/ready/metrics endpoints.
- [`live-stack-smoke.md`](live-stack-smoke.md) — manual acceptance checklist
  after upgrade or restore.
- [`data-backend-runbook.md`](data-backend-runbook.md) — bootstrap idempotency,
  engine-env contract, and re-running bootstrap.
- `crates/tdw-migration/src/lib.rs` — authoritative migration catalog and
  version list.
- `crates/tdw-bootstrap/src/main.rs` — bootstrap entry point, `ensure_schema()`
  call chain, and exit-code legend.
- `xtask/src/main.rs` — `migrate up/down/status` offline planning tool
  (`cargo run -p xtask --target-dir target -- migrate <up|down|status>`).
