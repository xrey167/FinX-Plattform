# Backup and restore runbook — `full`/`live` compose stack

Operational guide for backing up and restoring the five named volumes used by
the FinX-Plattform `full` and `live` Docker Compose profiles. Read in
conjunction with [`data-backend-runbook.md`](data-backend-runbook.md) (how the
stack comes live) and [`service-operability.md`](service-operability.md) (health
endpoints).

## State inventory

Each named volume holds a distinct kind of state. Understand what is
source-of-truth vs. derivable before deciding restore scope.

| Volume | Compose service | What lives there | Source-of-truth? |
|---|---|---|---|
| `postgres-data` | `postgres` | All Postgres data: `tdw_outbox`, `tdw_snapshot`, `tdw_bus`, `tdw_sessions*`, `tdw_rollout`, `system.worker_jobs`, and all domain tables applied by `tdw-bootstrap` schema calls | **Yes — primary source of truth** |
| `clickhouse-data` | `clickhouse` | All ClickHouse databases and tables (`tdw.*`, OLAP/analytics layer) | **Yes — primary for OLAP data**; the `tdw._tdw_bootstrap_marker` is re-created by bootstrap if missing |
| `qdrant-data` | `qdrant` | Vector collections (default: `tdw-default`) and their stored vectors | Derivable by re-ingesting from source, but expensive; treat as **source-of-truth** for vector index state |
| `meili-data` | `meilisearch` | Meilisearch index data (`tdw-default` and any application indexes) | Derivable by re-indexing from Postgres/ClickHouse; treat as **derived** — restore from source when possible |
| `minio-data` | `minio` | S3/MinIO objects in `tdw-default` bucket (blob store, the `_tdw_bootstrap_marker`) | **Yes** for any application blobs written there; the bootstrap marker alone is re-created by `docker compose run --rm tdw-bootstrap` |

**Restore order:** always restore durable stores first (Postgres, ClickHouse,
MinIO), then vector/lexical indexes (Qdrant, Meilisearch). Derived indexes can
be rebuilt from durable stores if a point-in-time snapshot is unavailable.

---

## Cold backup (compose down + volume snapshot)

The safest backup. Stop the stack so no writes race the copy.

### Stop the stack

```powershell
docker compose --profile live down
# Or for the full profile:
docker compose --profile full down
```

Verify all containers are stopped before proceeding:

```powershell
docker compose --profile live ps
```

### Snapshot each volume

Docker named volumes live under the Docker data root. The canonical way to
snapshot them without depending on the storage driver is to run a temporary
container that mounts the volume and tars it to a host path.

```powershell
# Create a backup directory on the host (adjust the path as needed).
$BACKUP_DIR = "C:\backup\finx-$(Get-Date -Format 'yyyyMMdd-HHmmss')"
New-Item -ItemType Directory -Force -Path $BACKUP_DIR

# Postgres
docker run --rm `
  -v finx-plattform_postgres-data:/data:ro `
  -v "${BACKUP_DIR}:/backup" `
  debian:bookworm-slim `
  tar -czf /backup/postgres-data.tar.gz -C /data .

# ClickHouse
docker run --rm `
  -v finx-plattform_clickhouse-data:/data:ro `
  -v "${BACKUP_DIR}:/backup" `
  debian:bookworm-slim `
  tar -czf /backup/clickhouse-data.tar.gz -C /data .

# Qdrant
docker run --rm `
  -v finx-plattform_qdrant-data:/data:ro `
  -v "${BACKUP_DIR}:/backup" `
  debian:bookworm-slim `
  tar -czf /backup/qdrant-data.tar.gz -C /data .

# Meilisearch
docker run --rm `
  -v finx-plattform_meili-data:/data:ro `
  -v "${BACKUP_DIR}:/backup" `
  debian:bookworm-slim `
  tar -czf /backup/meili-data.tar.gz -C /data .

# MinIO
docker run --rm `
  -v finx-plattform_minio-data:/data:ro `
  -v "${BACKUP_DIR}:/backup" `
  debian:bookworm-slim `
  tar -czf /backup/minio-data.tar.gz -C /data .
```

Record the git commit and image digests alongside the backup:

```powershell
git rev-parse HEAD > "$BACKUP_DIR\git-rev.txt"
docker compose --profile live images --format json > "$BACKUP_DIR\image-digests.json"
```

---

## Hot/online backup options (stack running)

Use native tools when stopping the stack is not acceptable. Each engine's tool
runs against the compose service on its published port.

### Postgres — `pg_dump` (logical) / `pg_basebackup` (physical)

**Logical dump** (portable, smaller, slower restore):

```powershell
# Dump the tdw database to a custom-format file (parallel-restore capable).
docker compose exec -T postgres `
  pg_dump -U tdw -d tdw -Fc -f /tmp/tdw.dump

# Copy out of the container.
docker compose cp postgres:/tmp/tdw.dump "$BACKUP_DIR\postgres-tdw.dump"
```

**Physical base backup** (faster restore, same Postgres major version required):

```powershell
docker compose exec -T postgres `
  pg_basebackup -U tdw -D /tmp/pgbase -Ft -z -P

docker compose cp postgres:/tmp/pgbase "$BACKUP_DIR\pgbase"
```

Note: `pg_basebackup` requires the `postgres` user to have the `REPLICATION`
privilege (the default `tdw` superuser in the compose stack has this). The `-Ft`
flag writes a tarball; `-z` compresses it. The result is a full cluster backup
compatible with `pg_restore` / point-in-time recovery.

### ClickHouse — `BACKUP TABLE` / `clickhouse-backup`

**`BACKUP TABLE` to a local path inside the container** (built-in, no extra
tooling):

```powershell
# Backup all tables in the tdw database to a named backup.
docker compose exec -T clickhouse `
  clickhouse-client `
    --user tdw --password tdw `
    --query "BACKUP DATABASE tdw TO Disk('default', 'tdw-backup-$(Get-Date -Format yyyyMMdd).zip')"

# The backup lands in ClickHouse's data directory (/var/lib/clickhouse/backup/).
# Copy it out:
docker compose cp "clickhouse:/var/lib/clickhouse/backup/" "$BACKUP_DIR\clickhouse-backup"
```

**Using `clickhouse-backup`** (third-party, supports incremental and remote
storage — see https://github.com/Altinity/clickhouse-backup for installation):

```bash
clickhouse-backup create tdw-$(date +%Y%m%d) \
  --config /etc/clickhouse-backup/config.yaml
```

### Qdrant — snapshot API

Qdrant exposes a REST snapshot API. Take a per-collection snapshot and download
it. The `tdw-default` collection is the bootstrap default; repeat for any
additional collections your application creates.

```powershell
# Trigger snapshot creation (returns a snapshot name).
$response = Invoke-RestMethod `
  -Method POST `
  -Uri "http://localhost:6333/collections/tdw-default/snapshots"
$snapshotName = $response.result.name

# Download the snapshot file.
Invoke-WebRequest `
  -Uri "http://localhost:6333/collections/tdw-default/snapshots/$snapshotName" `
  -OutFile "$BACKUP_DIR\qdrant-tdw-default-$snapshotName"
```

List existing snapshots:

```powershell
Invoke-RestMethod -Uri "http://localhost:6333/collections/tdw-default/snapshots"
```

### Meilisearch — dump API

Meilisearch creates a full dump (all indexes, settings, documents) via its REST
API.

```powershell
# Trigger dump creation; Meilisearch returns a task ID.
$task = Invoke-RestMethod `
  -Method POST `
  -Uri "http://localhost:7700/dumps"

# Poll until the task is complete (status = "succeeded").
do {
  Start-Sleep -Seconds 2
  $status = Invoke-RestMethod -Uri "http://localhost:7700/tasks/$($task.taskUid)"
} while ($status.status -notin @("succeeded","failed"))

if ($status.status -ne "succeeded") {
  Write-Error "Meilisearch dump failed: $($status | ConvertTo-Json)"
}

# The dump file lands inside the container at /meili_data/dumps/.
# Copy it out:
docker compose cp meilisearch:/meili_data/dumps/ "$BACKUP_DIR\meili-dumps"
```

### MinIO — `mc mirror`

Mirror the entire `tdw-default` bucket to a local directory using the MinIO
Client (`mc`). The `minio/mc` image is already pulled by compose.

```powershell
docker run --rm `
  --network finx-plattform_default `
  -v "${BACKUP_DIR}:/backup" `
  minio/mc:latest `
  sh -c '
    mc alias set local http://minio:9000 minio minio123 &&
    mc mirror local/tdw-default /backup/minio-tdw-default
  '
```

This copies all objects from the `tdw-default` bucket to
`$BACKUP_DIR\minio-tdw-default\` on the host.

---

## Restore procedures

### Prerequisites

Stop the live stack before restoring volumes (except for online Qdrant/Meili
snapshot restores — see their sections):

```powershell
docker compose --profile live down
```

### Restore order

1. **Postgres** (durable session, rollout, worker, and domain schemas)
2. **ClickHouse** (OLAP layer)
3. **MinIO** (blob store)
4. **Qdrant** (vector index — restore from snapshot or re-ingest)
5. **Meilisearch** (lexical index — restore from dump or re-index)

### Postgres restore

**From cold tarball:**

```powershell
# Drop and recreate the volume.
docker volume rm finx-plattform_postgres-data
docker volume create finx-plattform_postgres-data

# Untar the backup into the fresh volume.
docker run --rm `
  -v finx-plattform_postgres-data:/data `
  -v "${BACKUP_DIR}:/backup:ro" `
  debian:bookworm-slim `
  sh -c "cd /data && tar -xzf /backup/postgres-data.tar.gz"
```

**From `pg_dump` logical backup:**

```powershell
# Start only Postgres.
docker compose up -d postgres
# Wait for it to be healthy.
docker compose exec postgres pg_isready -U tdw -d tdw

# Restore into the existing database.
docker compose cp "$BACKUP_DIR\postgres-tdw.dump" postgres:/tmp/tdw.dump
docker compose exec -T postgres `
  pg_restore -U tdw -d tdw --clean --if-exists /tmp/tdw.dump
```

**Verification:**

```powershell
docker compose exec postgres psql -U tdw -d tdw -c "\dt"
# Expect: tdw_outbox, tdw_snapshot, tdw_bus, tdw_sessions,
#         tdw_sessions_permission_state, tdw_sessions_pending_approvals,
#         tdw_sessions_cost_ledger, tdw_rollout, system.worker_jobs
docker compose exec postgres psql -U tdw -d tdw `
  -c "SELECT count(*) FROM tdw_sessions_cost_ledger;"
```

### ClickHouse restore

**From cold tarball:**

```powershell
docker volume rm finx-plattform_clickhouse-data
docker volume create finx-plattform_clickhouse-data

docker run --rm `
  -v finx-plattform_clickhouse-data:/data `
  -v "${BACKUP_DIR}:/backup:ro" `
  debian:bookworm-slim `
  sh -c "cd /data && tar -xzf /backup/clickhouse-data.tar.gz"
```

**From `BACKUP TABLE` zip:**

```powershell
# Start ClickHouse only.
docker compose up -d clickhouse

# Copy the backup zip into the container and restore.
docker compose cp "$BACKUP_DIR\clickhouse-backup" clickhouse:/var/lib/clickhouse/backup/
docker compose exec -T clickhouse `
  clickhouse-client --user tdw --password tdw `
  --query "RESTORE DATABASE tdw FROM Disk('default', 'tdw-backup-YYYYMMDD.zip')"
```

**Verification:**

```powershell
docker compose exec clickhouse `
  clickhouse-client -u tdw --password tdw `
  --query "exists table tdw._tdw_bootstrap_marker"
# Returns 1.

docker compose exec clickhouse `
  clickhouse-client -u tdw --password tdw `
  --query "SELECT count() FROM tdw._tdw_bootstrap_marker"
```

### MinIO restore

**From cold tarball:**

```powershell
docker volume rm finx-plattform_minio-data
docker volume create finx-plattform_minio-data

docker run --rm `
  -v finx-plattform_minio-data:/data `
  -v "${BACKUP_DIR}:/backup:ro" `
  debian:bookworm-slim `
  sh -c "cd /data && tar -xzf /backup/minio-data.tar.gz"
```

**From `mc mirror` directory:**

```powershell
# Start MinIO and minio-init first.
docker compose up -d minio minio-init
# Wait for minio-init to complete (creates the tdw-default bucket).
docker compose wait minio-init

# Mirror the backup directory back into the bucket.
docker run --rm `
  --network finx-plattform_default `
  -v "${BACKUP_DIR}:/backup:ro" `
  minio/mc:latest `
  sh -c '
    mc alias set local http://minio:9000 minio minio123 &&
    mc mirror /backup/minio-tdw-default local/tdw-default
  '
```

**Verification:**

```powershell
docker compose exec minio `
  sh -c "mc alias set local http://localhost:9000 minio minio123 && mc ls local/tdw-default"
# Expect: _tdw_bootstrap_marker plus any application objects.
```

### Qdrant restore

**From snapshot file (online — Qdrant can be running):**

```powershell
# Start Qdrant.
docker compose up -d qdrant

# Upload the snapshot to restore the collection.
# If the collection already exists, delete it first to avoid conflicts:
Invoke-RestMethod `
  -Method DELETE `
  -Uri "http://localhost:6333/collections/tdw-default"

# Restore from snapshot file using the upload endpoint.
$snapshotFile = Get-Item "$BACKUP_DIR\qdrant-tdw-default-*.snapshot" | Select-Object -First 1
$form = @{ snapshot = Get-Item $snapshotFile.FullName }
Invoke-RestMethod `
  -Method POST `
  -Uri "http://localhost:6333/collections/tdw-default/snapshots/upload" `
  -Form $form
```

**From cold tarball (offline):**

```powershell
docker volume rm finx-plattform_qdrant-data
docker volume create finx-plattform_qdrant-data

docker run --rm `
  -v finx-plattform_qdrant-data:/data `
  -v "${BACKUP_DIR}:/backup:ro" `
  debian:bookworm-slim `
  sh -c "cd /data && tar -xzf /backup/qdrant-data.tar.gz"
```

**Verification:**

```powershell
Invoke-RestMethod -Uri "http://localhost:6333/collections/tdw-default" |
  Select-Object -ExpandProperty result |
  Select-Object status, vectors_count
# status should be "green"; vectors_count should match pre-backup value.
```

### Meilisearch restore

**From dump file (online — Meilisearch must be started with `--import-dump`):**

Meilisearch dump import requires a restart with a flag. The compose service does
not set `--import-dump` by default; use a one-off container:

```powershell
$dumpFile = Get-Item "$BACKUP_DIR\meili-dumps\*.dump" | Select-Object -First 1

# Copy the dump into the meili-data volume.
docker run --rm `
  -v finx-plattform_meili-data:/meili_data `
  -v "${BACKUP_DIR}:/backup:ro" `
  debian:bookworm-slim `
  sh -c "mkdir -p /meili_data/dumps && cp /backup/meili-dumps/*.dump /meili_data/dumps/"

# Start Meilisearch with the import flag to load the dump.
docker run --rm `
  -v finx-plattform_meili-data:/meili_data `
  -p 7700:7700 `
  getmeili/meilisearch:latest `
  meilisearch --import-dump /meili_data/dumps/$($dumpFile.Name) `
              --env development `
              --no-analytics
# Wait for Meilisearch to finish importing (it logs "Dump importation succeeded")
# then stop this container and restart the normal compose service.
docker compose up -d meilisearch
```

**From cold tarball:**

```powershell
docker volume rm finx-plattform_meili-data
docker volume create finx-plattform_meili-data

docker run --rm `
  -v finx-plattform_meili-data:/data `
  -v "${BACKUP_DIR}:/backup:ro" `
  debian:bookworm-slim `
  sh -c "cd /data && tar -xzf /backup/meili-data.tar.gz"
```

**Verification:**

```powershell
Invoke-RestMethod -Uri "http://localhost:7700/indexes/tdw-default" |
  Select-Object uid, numberOfDocuments
# uid = "tdw-default"; numberOfDocuments matches pre-backup value.
```

---

## Post-restore: bring the full stack live

After all volumes are restored, re-run bootstrap to ensure all engine schemas
and marker objects are present (it is idempotent):

```powershell
docker compose --profile live up -d --build
docker compose logs tdw-bootstrap
# Expect: {"step":"done","status":"ok","detail":"data backend live"}
```

Then verify the long-running services are healthy:

```powershell
# Daemon ops endpoint
curl -fsS http://localhost:9102/health

# MCP ops endpoint
curl -fsS http://localhost:9103/health

# Worker ops endpoint
curl -fsS http://localhost:9101/health
```

All three must return `200`. See [`service-operability.md`](service-operability.md)
for the full health/ready/metrics surface and graceful-drain behaviour.

---

## Test your restore drill

Run this drill periodically — at minimum before a major upgrade and after
significant data changes. Performing a restore only during an incident is a
false economy.

1. Take a cold backup following the steps above on a non-production copy of the
   stack (or a separate VM/directory).
2. Stop the copy stack: `docker compose --profile live down`.
3. Remove all five volumes (`docker volume rm finx-plattform_postgres-data` etc.).
4. Follow the restore procedures in restore order (Postgres → ClickHouse →
   MinIO → Qdrant → Meilisearch).
5. Run `docker compose --profile live up -d --build` and wait for bootstrap to
   complete.
6. Verify each engine's row/document/object counts match pre-backup values
   using the verification commands in each section above.
7. Run the live-stack smoke checklist:
   [`live-stack-smoke.md`](live-stack-smoke.md).

A drill that completes steps 1–7 cleanly is your backup validity proof.

---

## See also

- [`data-backend-runbook.md`](data-backend-runbook.md) — bringing the `live`
  stack up from scratch, bootstrap idempotency, and engine-env contract.
- [`service-operability.md`](service-operability.md) — daemon, MCP, and worker
  health/ready/metrics endpoints.
- [`live-stack-smoke.md`](live-stack-smoke.md) — manual acceptance checklist
  after any restore or upgrade.
- [`upgrade-runbook.md`](upgrade-runbook.md) — upgrading TDW images and schema
  migrations; always back up first.
- `crates/tdw-bootstrap/src/main.rs` — exit-code legend and per-step JSON
  shape for bootstrap.
