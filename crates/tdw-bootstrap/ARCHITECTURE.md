# tdw-bootstrap architecture

`tdw-bootstrap` is a sequential, fail-fast init binary. It runs a fixed series of
"bring the backend live" steps, logging one JSON line each and exiting on the
first failure with a step-specific code.

## Module map

| Path | Contents |
| --- | --- |
| `src/main.rs` | the `tdw-bootstrap` binary: step sequence, `require_env`, `log_step` |

## Step sequence and exit codes

```text
start
  ├─ env            (read required vars)                       fail → 2
  ├─ postgres-connect (PgEngine::connect)                      fail → 3
  ├─ outbox-schema    (PgOutboxStore::ensure_schema)           fail → 4
  ├─ snapshot-schema  (PgSnapshotStore::ensure_schema)         fail → 4
  ├─ bus-schema       (PgEventBus::ensure_schema)              fail → 4
  ├─ session-schema   (PgSessionStore::ensure_schema)          fail → 4
  ├─ s3-marker        (S3Engine.put_object marker)             fail → 5
  ├─ s3-roundtrip     (get_object == written body)             fail → 6
  ├─ clickhouse-schema  (if TDW_CLICKHOUSE_URL set)            fail → 7
  ├─ qdrant-collection  (if TDW_QDRANT_URL set)                fail → 8
  ├─ meili-index        (if TDW_MEILI_URL set)                 fail → 9
  └─ done
```

Postgres + S3 are mandatory; ClickHouse/Qdrant/Meilisearch are each skipped
unless their `*_URL` is set, so the minimal Postgres + S3 bootstrap keeps working
unchanged.

## Constants (release-runbook contract)

`MARKER_KEY = "_tdw_bootstrap_marker"`, `MARKER_BODY = "tdw-bootstrap ok\n"`,
`CLICKHOUSE_DB = "tdw"`, `QDRANT_COLLECTION = "tdw-default"`,
`MEILI_INDEX = "tdw-default"`, `DEFAULT_QDRANT_VECTOR_SIZE = 1536`. These are
asserted by the crate's own tests and referenced by the deployment runbook.

## Output

Each step calls `log_step(step, status, detail?)`, printing a JSON object like
`{"step":"postgres-connect","status":"ok"}` to stdout. The S3-marker failure
includes a hint that the bucket must exist (the compose `minio-init` service
creates it).

## Security posture

- **Connection details come from the environment only** — no secret is hard-coded;
  `require_env` fails closed (exit 2) with the missing variable name.
- **Fail-fast**: any step error stops the run with a precise exit code, so a
  half-provisioned backend never silently proceeds.
- It writes only a small marker object and idempotent `create … if not exists`
  schema statements; re-running is safe.
- Requires live backends — it is **not** offline (the bundled example is the
  offline config-inspection demo).

## Integration points

- `tdw-storage-postgres` / `tdw-bus` / `tdw-outbox` / `tdw-session` /
  `tdw-snapshot` — the Postgres schema steps.
- `tdw-storage-s3` — the marker write/roundtrip.
- `tdw-storage-clickhouse` / `tdw-storage-qdrant` / `tdw-storage-meilisearch` —
  the optional baseline schema/collection/index steps.
