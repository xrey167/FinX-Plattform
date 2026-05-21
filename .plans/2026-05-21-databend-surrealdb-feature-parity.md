# FinX-Finance — Databend + SurrealDB Feature Parity (Layer C)

**Project:** FinX-Finance (`C:\Users\ReyDa\FinX-Finance\`)
**Date:** 2026-05-21
**Mode:** `/omc-plan --direct` (extension to v2.1 + data-engineering+agent-schemas)
**Status:** Draft — Phases 9–13 added on top of Phases 0–8
**Parent plans:**
- [`2026-05-21-rust-trading-data-warehouse.md`](./2026-05-21-rust-trading-data-warehouse.md) — Phases 0–6 (core warehouse)
- [`2026-05-21-data-engineering-and-agent-schemas.md`](./2026-05-21-data-engineering-and-agent-schemas.md) — Phases 7–8 (data eng + agent schemas)

---

## 1. Goal

The parent plans intentionally **SKIP** Databend and SurrealDB as runtime components (verdict §6 of the core plan). This extension reverses the framing: **adopt their feature surface without adopting their code.**

We replicate every meaningful capability — time travel, streams/CDC, graph edges, live queries, UDFs, masking, stages, open table formats, declarative schema, geometry — on top of our specialist stack (ClickHouse + Postgres + Qdrant + Meilisearch + S3 + RiverQueue + dbt + Rust runtime). The result: a Rust trading warehouse that does *what* Databend + SurrealDB do, while keeping the clean-room boundary (§0 of core plan) and the specialist storage matrix.

This plan enumerates **438 distinct features** across both systems (210 Databend + 228 SurrealDB), maps each to **Covered / Partial / Gap / OOS**, and proposes 5 new phases (9–13) + ~9 new crates to close the gaps.

---

## 2. Honest non-goals (will not replicate)

| Source | Feature | Why we skip |
|--------|---------|------------|
| Databend | Multi-cluster shared storage (warehouses, workload groups) | Personal use; one machine. |
| Databend | Cloud billing / tenant isolation | N/A for personal. |
| Databend | Databend Cloud web console | N/A. |
| Databend | MySQL wire protocol | We expose Postgres + HTTP + gRPC + MCP. |
| Databend | HDFS, IPFS as primary storage | S3/MinIO is enough. |
| Databend | Snowflake-compatible driver shim | N/A. |
| SurrealDB | Browser WASM / IndexedDB engine | Server-side only. |
| SurrealDB | TiKV / FoundationDB distributed backend | OOS. |
| SurrealDB | Mobile builds | OOS. |
| SurrealDB | Surrealist web UI | Use DBeaver / DataGrip / open-db-studio. |
| SurrealDB | Multi-tenancy (namespace + database hierarchy) | Single-tenant. We expose Postgres schemas which already partition. |
| Both | Custom query language (SurrealQL / Databend SQL extensions) | We expose Postgres SQL + ClickHouse SQL + Rust traits. **No new query language.** |

**Critical decision (§7 ADR):** we do **not** build a new query language. SurrealQL's expressive surface is genuinely beautiful but a new DSL is years of work; we expose the same capabilities via typed Rust APIs + extended PG/CH SQL.

---

## 3. Layer C — feature inventory & status

Notation: ✓ = Covered (already in parent plans) · ◐ = Partial (in plans but needs extension) · ✗ = Gap (new work, owns a phase) · — = OOS (§2)

### 3.1 Databend feature mapping (210 features)

#### 3.1.1 Storage Architecture (16)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-1  | Fuse table engine (snapshot/segment/block tree) | ✗ | `tdw-snapshot` | New snapshot layer over ClickHouse + Parquet/S3. |
| D-2  | Snapshot tree (every write → new snapshot ID) | ✗ | `tdw-snapshot` | P0 — enables time travel. |
| D-3  | Segment layer with pruning metadata (min/max, null, distinct) | ◐ | `tdw-snapshot` + CH | CH partitions cover most of this. |
| D-4  | Block layer (Parquet-encoded data files) | ◐ | `tdw-storage-parquet` | Already in parent plan. |
| D-5  | Cluster key (`CLUSTER BY`) | ✓ | CH `ORDER BY` | Native. |
| D-6  | Transient table (no history) | ✗ | `tdw-snapshot` | Table flag: skip snapshot writes. |
| D-7  | External table (read-only over stage files) | ◐ | `tdw-stage` + CH S3 engine | Surface via stage abstraction. |
| D-8  | Attach table (mount foreign snapshot path) | — | — | Niche; revisit if needed. |
| D-9  | Snapshot tags (named retained pointers) | ✗ | `tdw-snapshot` | Git-tag semantics over snapshot IDs. |
| D-10 | Random table engine (synthetic rows) | ✓ | `tdw-test-utils` | Already covered. |
| D-11 | Storage backends: S3 | ✓ | `tdw-storage-s3` | |
| D-12 | Storage backends: GCS, Azure Blob, OSS, COS | ◐ | `tdw-storage-s3` | Add via opendal/aws-sdk; one PR each. |
| D-13 | Compression codecs (gzip, zstd, snappy, lz4, brotli) | ✓ | CH + Parquet | Native both. |
| D-14 | File formats (Parquet, CSV, TSV, NDJSON, JSON, ORC, AVRO, XML) | ◐ | `tdw-stage` | Parquet/CSV/JSON ✓; ORC/AVRO/XML gap. |
| D-15 | Auto schema evolution on COPY | ✗ | `tdw-stage` | New column → ADD COLUMN automatically. |
| D-16 | Encryption-at-rest (SSE-KMS) | ✓ | S3 | Native S3. |

#### 3.1.2 Query Engine (16)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-17 | Vectorized columnar execution | ✓ | CH | Native. |
| D-18 | MPP / distributed execution | ◐ | CH cluster | Single-node default; cluster opt-in. |
| D-19 | Cost-based optimizer with join reordering | ✓ | CH + PG | Both. |
| D-20 | Predicate pushdown to scan | ✓ | CH + PG | |
| D-21 | Column pruning | ✓ | CH | |
| D-22 | Partition pruning | ✓ | CH | |
| D-23 | Bloom filter pruning | ✓ | CH bloom index | |
| D-24 | Query result cache | ✓ | CH | |
| D-25 | Table-level data cache (LRU disk) | ✓ | CH | |
| D-26 | Distributed shuffle (Arrow Flight) | ◐ | CH cluster | OOS in single-node. |
| D-27 | Streaming aggregation | ✓ | CH AggregatingMergeTree | |
| D-28 | Parallel scan with degree control | ✓ | CH `max_threads` | |
| D-29 | Aggregating index (precomputed roll-ups + rewrite) | ✓ | CH AggregatingMergeTree + Projection | |
| D-30 | Materialized view query rewrite (auto) | ✗ | `tdw-rewrite` (sub-module of `tdw-runtime`) | Optional; falls back to manual. |
| D-31 | SAMPLE BLOCK / SAMPLE ROW | ✓ | CH SAMPLE / PG TABLESAMPLE | |
| D-32 | Memory spilling to disk for joins/aggs | ✓ | CH | |

#### 3.1.3 SQL Surface (16)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-33 | DDL coverage (CREATE/ALTER/DROP/TRUNCATE/RENAME/UNDROP/FLASHBACK/REPLACE) | ◐ | CH + PG + `tdw-snapshot` | UNDROP/FLASHBACK = gap (D-50/51). |
| D-34 | DML (INSERT/UPDATE/DELETE/REPLACE/MERGE/COPY INTO/TRUNCATE) | ◐ | CH + PG | MERGE in PG/CH; REPLACE INTO covered by ON CONFLICT. |
| D-35 | DCL (GRANT/REVOKE/SHOW GRANTS/SET ROLE) | ✓ | PG | Native. |
| D-36 | INSERT MULTI (single scan, multiple targets) | ✗ | `tdw-runtime` | New helper in CommandRunner. |
| D-37 | JOIN types: INNER, LEFT, RIGHT, FULL, CROSS, LATERAL | ✓ | PG + CH | |
| D-38 | ASOF JOIN | ✓ | CH ASOF | PG via lateral + LIMIT. |
| D-39 | SEMI/ANTI joins | ✓ | both via EXISTS/NOT EXISTS | |
| D-40 | WITH (CTE, recursive) | ✓ | both | |
| D-41 | QUALIFY clause | ◐ | CH ✓; PG via subquery | |
| D-42 | PIVOT / UNPIVOT | ◐ | CH crosstab; PG `tablefunc` | |
| D-43 | ROLLUP / CUBE / GROUPING SETS | ✓ | PG; CH ◐ | |
| D-44 | Set ops (UNION ALL/INTERSECT/EXCEPT) | ✓ | both | |
| D-45 | Window functions with named windows | ✓ | both | |
| D-46 | Stored procedure scripting (variables, IF/LOOP/WHILE, dynamic SQL) | ◐ | PG plpgsql | Cover via plpgsql + `tdw-runtime` orchestration. |
| D-47 | Multi-statement transactions (BEGIN/COMMIT/ROLLBACK) | ✓ | PG | |
| D-48 | Session variables (`SET VARIABLE`, `$var`) | ✓ | both | |

#### 3.1.4 Time Travel & Versioning (11)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-49 | `SELECT ... AT (SNAPSHOT => '<id>')` | ✗ | `tdw-snapshot` | **P0** — flagship feature. |
| D-50 | `SELECT ... AT (TIMESTAMP => '<ts>')` | ✗ | `tdw-snapshot` | P0. |
| D-51 | `SELECT ... AT (OFFSET => -N)` (relative seconds) | ✗ | `tdw-snapshot` | P0. |
| D-52 | `SELECT ... AT (STREAM => <name>)` | ✗ | `tdw-snapshot` + `tdw-stream` | Cross-cutting with streams. |
| D-53 | `UNDROP TABLE` / `UNDROP DATABASE` | ✗ | `tdw-snapshot` | Soft-delete with TTL. |
| D-54 | `FLASHBACK TABLE TO SNAPSHOT/TIMESTAMP` | ✗ | `tdw-snapshot` | P0. |
| D-55 | `CREATE TABLE LIKE … AT (SNAPSHOT => …)` (fork) | ✗ | `tdw-snapshot` | "Data branching". |
| D-56 | Snapshot tags (`CREATE TAG`) | ✗ | `tdw-snapshot` | Named pointers. |
| D-57 | Per-table `DATA_RETENTION_TIME_IN_DAYS` | ✗ | `tdw-snapshot` | TOML/SQL-level config. |
| D-58 | Fail-safe (post-retention recovery) | — | — | Enterprise Databend feature, OOS. |
| D-59 | `VACUUM` (TABLE / DROP TABLE / TEMP FILES) | ✗ | `tdw-snapshot` | Snapshot GC. |

#### 3.1.5 Streams / CDC (10)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-60 | `CREATE STREAM ON TABLE` | ✗ | `tdw-stream` | **P0** — flagship feature. |
| D-61 | `APPEND_ONLY = TRUE` (INSERT-only tracking) | ✗ | `tdw-stream` | P0. |
| D-62 | Standard mode (INSERT/UPDATE/DELETE) | ✗ | `tdw-stream` | P0. |
| D-63 | `AT (SNAPSHOT|TIMESTAMP|OFFSET|STREAM => …)` start | ✗ | `tdw-stream` | |
| D-64 | Metadata columns (`change$action`, `change$is_update`, `change$row_id`) | ✗ | `tdw-stream` | |
| D-65 | Transactional offset advance on consume | ✗ | `tdw-stream` | Postgres `FOR UPDATE` + advisory locks. |
| D-66 | Stream-to-stream cloning | ✗ | `tdw-stream` | |
| D-67 | Stream output composable with MERGE/INSERT | ✗ | `tdw-stream` | |
| D-68 | `DESC STREAM` / `SHOW STREAMS` | ✗ | `tdw-stream` | System table view. |
| D-69 | Stream retention bounded by source-table retention | ✗ | `tdw-stream` | Coupled with snapshot retention. |

#### 3.1.6 Tasks (Scheduled SQL) (10)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-70 | `CREATE TASK ... SCHEDULE = N MINUTE | CRON` | ✓ | `tdw-pipeline` + RiverQueue | Already in plan. |
| D-71 | `AFTER <task[,...]>` (DAG chains) | ✓ | `tdw-pipeline` | Already in plan. |
| D-72 | `WHEN <bool-expr>` (predicate gate) | ✗ | `tdw-pipeline` | Add predicate eval to job descriptor. |
| D-73 | `ERROR_INTEGRATION` (failure webhooks) | ✗ | `tdw-notify` | New crate (sub-module of `tdw-runtime`). |
| D-74 | `SUSPEND_TASK_AFTER_NUM_FAILURES = N` | ✓ | RiverQueue retry policy | |
| D-75 | `EXECUTE TASK` (manual run) | ✓ | `tdw-cli` | |
| D-76 | `ALTER TASK SUSPEND/RESUME/MODIFY WHEN/ADD AFTER` | ✗ | `tdw-pipeline` | Add ALTER ops. |
| D-77 | Multi-statement task body | ✓ | RiverQueue | |
| D-78 | Task history (`system.task_history`) | ✓ | RiverQueue history | |
| D-79 | `SHOW TASKS` | ✓ | RiverQueue + view | |

#### 3.1.7 Stages & External Integration (12)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-80 | User stage (`@~`) | ✗ | `tdw-stage` | Per-user S3 prefix. |
| D-81 | Internal named stage (`CREATE STAGE`) | ✗ | `tdw-stage` | DB-managed bucket path. |
| D-82 | External stage (URL=s3://) | ✗ | `tdw-stage` | |
| D-83 | File-format binding | ✗ | `tdw-stage` | |
| D-84 | Stage-level COPY_OPTIONS (ON_ERROR, SIZE_LIMIT, MAX_FILES, PURGE, FORCE) | ✗ | `tdw-stage` | |
| D-85 | Stage encryption (SSE-KMS pass-through) | ✓ | `tdw-storage-s3` | |
| D-86 | `LIST @stage` / `REMOVE @stage` | ✗ | `tdw-stage` | |
| D-87 | `PRESIGN @stage/file` (presigned URLs) | ✓ | `BlobEngine::presigned_url` | Already in `tdw-storage-s3`. |
| D-88 | CLI PUT / GET (client-side upload/download) | ◐ | `tdw-cli` | Add `tdw-cli stage put/get`. |
| D-89 | Direct query over stage: `SELECT FROM @stage` | ✗ | `tdw-stage` + CH S3 engine | |
| D-90 | Inline credential URI in COPY INTO | ✗ | `tdw-stage` | |
| D-91 | PCRE2 regex `PATTERN` for file matching | ✗ | `tdw-stage` | `regex` crate. |

#### 3.1.8 Materialized Views (6)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-92 | `CREATE MATERIALIZED VIEW` (stored, rewritable) | ✓ | CH MV + dbt incremental | |
| D-93 | `REFRESH MODE = AUTO | MANUAL` | ✓ | CH (always-fresh) + dbt schedule | |
| D-94 | `INITIALIZE = ON_CREATE | ON_SCHEDULE` | ◐ | dbt run --select | |
| D-95 | Auto query rewrite | ✗ | `tdw-rewrite` (optional) | Falls back to manual. |
| D-96 | `REFRESH MATERIALIZED VIEW <name>` | ✓ | CH `OPTIMIZE` / dbt `--full-refresh` | |
| D-97 | `ALTER MATERIALIZED VIEW SUSPEND/RESUME` | ✗ | `tdw-pipeline` | Suspend dbt schedule. |

#### 3.1.9 UDFs (10)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-98 | SQL scalar UDF | ✓ | PG `CREATE FUNCTION`; CH UDF | |
| D-99 | Python scalar UDF (with PACKAGES) | ✗ | `tdw-udf-python` | Sandboxed subprocess (RustPython or PyO3). |
| D-100 | JavaScript scalar UDF | ✗ | `tdw-udf-js` | QuickJS via `rquickjs` crate. |
| D-101 | WebAssembly UDF (Arrow UDF binary) | ✗ | `tdw-udf-wasm` | wasmtime crate. |
| D-102 | Aggregate UDF (UDAF) Python + JS | ✗ | `tdw-udf` | |
| D-103 | Table UDF (returns result set) | ◐ | PG SETOF | |
| D-104 | External Function (HTTP / Arrow Flight) | ✗ | `tdw-udf-external` | |
| D-105 | Sandboxed UDF (isolated worker for agent code) | ✗ | `tdw-udf` | Process isolation + resource limits. |
| D-106 | Stored Procedure (imperative scripting) | ✓ | PG plpgsql | |
| D-107 | `ALTER FUNCTION` / `DROP FUNCTION` / `SHOW USER FUNCTIONS` | ✓ | PG + CH | |

#### 3.1.10 Security (12)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-108 | RBAC (USER, ROLE, default + secondary, SET ROLE) | ✓ | PG roles | Native. |
| D-109 | Predefined roles (`account_admin`, `public`) | ✓ | PG bootstrap | One-time. |
| D-110 | Object ownership transferable | ✓ | PG `REASSIGN OWNED` | |
| D-111 | Privilege scopes per object | ✓ | PG GRANT | |
| D-112 | Password Policy (length, complexity, rotation) | ◐ | PG `passwordcheck` ext | Configure in Phase 13. |
| D-113 | Network Policy (IP allow/block) | ✓ | PG `pg_hba.conf` | |
| D-114 | Masking Policy (column rewrite by role) | ✗ | `tdw-mask` (sub-module) | New layer; CH/PG don't have native. |
| D-115 | Row Access Policy | ✓ | PG RLS | Native. CH has limited. |
| D-116 | Audit Trail (`system.query_log`) | ✓ | PG `pg_audit` + CH `system.query_log` | |
| D-117 | TLS encryption in transit | ✓ | both | |
| D-118 | Encryption at rest | ✓ | S3 SSE | |
| D-119 | Connection objects (named credential blobs) | ✗ | `tdw-auth` | Like Snowflake "external integrations". |

#### 3.1.11 Connectivity (15)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-120 | Native CLI with PUT/GET | ◐ | `tdw-cli` | Already in plan; add stage ops. |
| D-121 | MySQL wire protocol | — | — | OOS (§2). |
| D-122 | ClickHouse HTTP handler | ✓ | CH native | |
| D-123 | HTTP REST API | ✓ | `tdw-service` axum | |
| D-124 | JDBC driver | — | — | OOS. |
| D-125 | Python driver | ◐ | Postgres `psycopg` + tdw HTTP | |
| D-126 | Go driver | ◐ | PG drivers + HTTP | |
| D-127 | Node.js driver | ◐ | PG drivers + HTTP | |
| D-128 | Rust driver | ✓ | sqlx + tdw-core embedded | |
| D-129 | MCP server | ✓ | `tdw-mcp` | |
| D-130 | DBeaver / DataGrip GUI | ✓ | PG wire | |
| D-131 | BI tools (Tableau/Power BI/Superset/Metabase/Grafana) | ✓ | PG wire | |
| D-132 | Jupyter / Deepnote / Hex | ✓ | PG wire | |
| D-133 | dbt | ✓ | dbt-postgres + dbt-clickhouse | Phase 7. |
| D-134 | Connectors (Kafka, Flink, Spark, Debezium, Airbyte) | ◐ | external | Document compatibility; not built-in. |

#### 3.1.12 Object Storage Support (8)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-135 | AWS S3 | ✓ | `tdw-storage-s3` | |
| D-136 | GCS | ◐ | `tdw-storage-s3` ext | One PR. |
| D-137 | Azure Blob | ◐ | `tdw-storage-s3` ext | |
| D-138 | Alibaba OSS, Tencent COS | ◐ | `tdw-storage-s3` ext | |
| D-139 | HDFS | — | — | OOS. |
| D-140 | MinIO (S3-compat) | ✓ | `tdw-storage-s3` | |
| D-141 | Local `fs://` | ✓ | `tdw-storage-fs` (sub) | Easy add. |
| D-142 | IPFS / Hugging Face datasets | ◐ | `tdw-provider-huggingface` (P2) | As providers, not storage backends. |

#### 3.1.13 Table Types (9)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-143 | Fuse (default) | ✗ | `tdw-snapshot` | Maps to CH `ReplacingMergeTree` + snapshot tree. |
| D-144 | Transient (no history) | ✗ | `tdw-snapshot` | Table flag. |
| D-145 | External (read-only over stage) | ✗ | `tdw-stage` | |
| D-146 | Attach | — | — | OOS. |
| D-147 | Random (synthetic) | ✓ | `tdw-test-utils` | |
| D-148 | Iceberg (read) | ✗ | `tdw-table-format` | P1. |
| D-149 | Delta Lake (read) | ✗ | `tdw-table-format` | P1. |
| D-150 | Hive (read via metastore) | — | — | OOS. |
| D-151 | Snapshot table (read-only handle) | ✗ | `tdw-snapshot` | |

#### 3.1.14 Open Table Formats (5)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-152 | Iceberg via catalog (REST/Glue/Polaris/Hive) | ✗ | `tdw-table-format` | P1; `iceberg-rust` crate. |
| D-153 | Iceberg time travel + partition + schema evolution | ✗ | `tdw-table-format` | |
| D-154 | Delta Lake reads | ✗ | `tdw-table-format` | `delta-rs` crate. |
| D-155 | Hive Metastore catalog | — | — | OOS. |
| D-156 | Direct Parquet/ORC/AVRO over stage | ◐ | `tdw-stage` + CH | Parquet covered; ORC/AVRO ◐. |

#### 3.1.15 Performance Features (20)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-157 | Cluster key | ✓ | CH ORDER BY | |
| D-158 | Auto recluster | ✓ | CH OPTIMIZE | |
| D-159 | Bloom filter index | ✓ | CH bloom | |
| D-160 | Ngram index (LIKE accel) | ✓ | CH `ngrambf_v1` | |
| D-161 | Inverted full-text index (BM25) | ✓ | Meilisearch + CH inverted | |
| D-162 | Vector index (HNSW) | ✓ | Qdrant | |
| D-163 | Spatial index | ◐ | PostGIS + CH spatial | Phase 11. |
| D-164 | Aggregating index | ✓ | CH AggregatingMergeTree | |
| D-165 | Virtual columns (auto-extracted JSON fields) | ✓ | CH `MATERIALIZED` columns | |
| D-166 | Materialized views with rewrite | ◐ | CH + dbt; auto-rewrite gap | |
| D-167 | Column pruning | ✓ | CH | |
| D-168 | Predicate pushdown | ✓ | CH + PG | |
| D-169 | Partition pruning | ✓ | CH | |
| D-170 | Bloom-block pruning | ✓ | CH | |
| D-171 | Query result cache | ✓ | CH | |
| D-172 | Block data cache | ✓ | CH | |
| D-173 | Table meta cache | ✓ | CH | |
| D-174 | Distributed shuffle | ◐ | CH cluster | |
| D-175 | Memory spill | ✓ | CH | |
| D-176 | Statistics (NDV, min/max, null) per segment | ✓ | CH + PG ANALYZE | |

#### 3.1.16 Operations (12)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-177 | `OPTIMIZE TABLE COMPACT` | ✓ | CH OPTIMIZE FINAL | |
| D-178 | `OPTIMIZE TABLE PURGE` | ✗ | `tdw-snapshot` | Snapshot GC. |
| D-179 | `ALTER TABLE RECLUSTER` | ✓ | CH OPTIMIZE | |
| D-180 | VACUUM TABLE / DROP TABLE / TEMP FILES / VIRTUAL COLUMN | ✗ | `tdw-snapshot` | |
| D-181 | `ANALYZE TABLE` | ✓ | PG + CH | |
| D-182 | Full-cluster backup CLI | ✓ | `pg_dump` + `clickhouse-backup` | Wrap in `tdw-cli backup`. |
| D-183 | `FLUSH PRIVILEGES` | ✓ | PG | |
| D-184 | `KILL QUERY` / `KILL CONNECTION` | ✓ | both | |
| D-185 | `SHOW PROCESSLIST` | ✓ | both | |
| D-186 | `SHOW LOCKS` | ✓ | PG `pg_locks` | |
| D-187 | Per-table `DATA_RETENTION_TIME_IN_DAYS` | ✗ | `tdw-snapshot` | TOML/SQL config. |
| D-188 | Manual snapshot creation/deletion | ✗ | `tdw-snapshot` | |

#### 3.1.17 Observability (10)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-189 | `system.query_log` (historical queries) | ✓ | CH + PG `pg_stat_statements` | |
| D-190 | `system.processes` | ✓ | both | |
| D-191 | `system.clusters` | ◐ | CH | Single-node default. |
| D-192 | `system.metrics` (internal counters) | ✓ | CH | |
| D-193 | `system.tables/columns/databases/schemas` | ✓ | both | |
| D-194 | `system.streams/tasks/stages/functions` | ✗ | `tdw-snapshot` + others | Once features land. |
| D-195 | Prometheus metrics endpoint | ✓ | tdw-service + CH/PG exporters | |
| D-196 | OpenTelemetry tracing | ✓ | `tracing` + `opentelemetry-otlp` crate | |
| D-197 | EXPLAIN variants (AST, RAW, ANALYZE, GRAPHICAL, PERF, SYNTAX) | ◐ | PG + CH EXPLAIN | All but GRAPHICAL/PERF native. |
| D-198 | `system.backtrace` | ◐ | PG `pg_backend_pid()` + RUST_BACKTRACE | |

#### 3.1.18 Data Types (15)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-199 | TINYINT/SMALLINT/INT/BIGINT (signed + unsigned) | ✓ | both | |
| D-200 | FLOAT / DOUBLE | ✓ | both | |
| D-201 | DECIMAL up to P=76 | ✓ | PG NUMERIC; CH Decimal256 | |
| D-202 | BOOLEAN | ✓ | both | |
| D-203 | VARCHAR / STRING | ✓ | both | |
| D-204 | BINARY / VARBINARY | ✓ | PG bytea; CH `String` | |
| D-205 | DATE | ✓ | both | |
| D-206 | TIMESTAMP / TIMESTAMP_TZ | ✓ | both | |
| D-207 | INTERVAL | ✓ | PG; CH limited | |
| D-208 | ARRAY<T> | ✓ | both | |
| D-209 | MAP<K,V> | ✓ | PG hstore/JSONB; CH Map | |
| D-210 | TUPLE | ✓ | CH; PG via composite type | |
| D-211 | VARIANT (JSON) | ✓ | PG JSONB; CH JSON | |
| D-212 | BITMAP (Roaring) | ✓ | CH AggregateFunction(groupBitmap) | |
| D-213 | VECTOR<F32, N> | ✓ | Qdrant; CH ARRAY(Float32) + vector index | |
| D-214 | GEOMETRY / GEOGRAPHY (WKB/EWKB) | ◐ | PostGIS + CH spatial | Phase 11. |

#### 3.1.19 Functions (15 categories)

All listed function families have direct CH+PG equivalents, except as noted:

| # | Category | Status | Notes |
|---|----------|--------|-------|
| D-215 | Numeric / Math (trig, log, ceil/floor, etc.) | ✓ | both |
| D-216 | String (regex, trim, replace, pad, levenshtein) | ✓ | both |
| D-217 | Date / Time (DATE_PART, DATE_TRUNC, DATEADD, timezone) | ✓ | both |
| D-218 | Conversion (CAST, TRY_CAST, `::`) | ✓ | both |
| D-219 | Conditional (CASE, COALESCE, NULLIF, IF, GREATEST, LEAST) | ✓ | both |
| D-220 | Aggregate (SUM/COUNT/AVG/MIN/MAX/MEDIAN/QUANTILE/ARRAY_AGG/HLL) | ✓ | both |
| D-221 | Window (LAG/LEAD/RANK/NTILE/CUME_DIST + frames) | ✓ | both |
| D-222 | Semi-structured (JSON/Array/Map/Tuple/FLATTEN/PARSE_JSON) | ✓ | both |
| D-223 | Full-text (MATCH, SCORE, BM25) | ✓ | Meilisearch + CH inverted |
| D-224 | Vector (cosine/l1/l2/inner_product/dims/norm) | ✓ | Qdrant + CH ARRAY funcs |
| D-225 | Geospatial (ST_DISTANCE, ST_CONTAINS, GeoHash, H3 grid) | ◐ | PostGIS + h3-rs |
| D-226 | Bitmap (bitmap_count/or/and/xor/contains/subset) | ✓ | CH |
| D-227 | Hash (MD5/SHA1/SHA2/CityHash64/XXH3) | ✓ | both |
| D-228 | UUID (gen_random_uuid, uuid_zero) | ✓ | both |
| D-229 | IP / CIDR | ✓ | PG inet/cidr; CH IPv4/IPv6 |

#### 3.1.20 Cluster Mode (10)

All cluster features marked OOS for personal scope (§2):

| # | Feature | Status |
|---|---------|--------|
| D-230 | databend-meta Raft | — |
| D-231 | databend-query stateless nodes | — |
| D-232 | Warehouse abstraction | — |
| D-233 | Workload Group (CPU/memory limits) | — |
| D-234 | Worker / physical assignment | — |
| D-235 | Multi-warehouse shared storage | — |
| D-236 | Auto-scale / pause / resume | — |
| D-237 | Arrow Flight inter-node | — |
| D-238 | Tenant isolation | — |
| D-239 | `USE WAREHOUSE` / `SHOW WAREHOUSES` | — |

#### 3.1.21 Notebook / App (7)

| # | Feature | Status | Notes |
|---|---------|--------|-------|
| D-240 | Databend Cloud console | — | OOS. |
| D-241 | Web SQL workbench | ◐ | Optional UI later. |
| D-242 | Notebook integration (Jupyter/Hex/Deepnote) | ✓ | via PG wire. |
| D-243 | BI integration | ✓ | via PG wire. |
| D-244 | dbt-databend adapter | ✓ | dbt-postgres + dbt-clickhouse equivalent. |
| D-245 | MCP server | ✓ | `tdw-mcp`. |
| D-246 | Embedded SQL playgrounds in docs | ◐ | mdBook + sqlx examples. |

#### 3.1.22 Distinctive / Misc (14)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| D-247 | Data branching (git-like fork/replace) | ✗ | `tdw-snapshot` | P0. |
| D-248 | Sandbox UDF for AI-agent code | ✗ | `tdw-udf` | P1. |
| D-249 | Auto schema evolution on COPY | ✗ | `tdw-stage` | P0. |
| D-250 | Pluggable AI/ML via Arrow Flight | ✗ | `tdw-udf-external` | P1. |
| D-251 | Stored-procedure scripting | ✓ | PG plpgsql | |
| D-252 | Pipes (auto-ingest from stage) | ✗ | `tdw-pipe` | P1. |
| D-253 | Dictionary objects (in-memory KV lookup) | ✓ | Redis + sqlx prepared | |
| D-254 | Sequences | ✓ | PG | |
| D-255 | Notifications | ✗ | `tdw-notify` | P0 (also covers D-73). |
| D-256 | Tags (governance metadata k/v) | ✗ | `tdw-tags` (sub-module) | P0. |
| D-257 | Virtual columns | ✓ | CH MATERIALIZED | |
| D-258 | Aggregating Index | ✓ | CH | |
| D-259 | Multi-table INSERT | ✗ | `tdw-runtime` | |
| D-260 | `REPLACE INTO ... ON (key)` | ✓ | PG ON CONFLICT + CH ReplacingMergeTree | |
| D-261 | `EXECUTE IMMEDIATE` (dynamic SQL) | ✓ | PG plpgsql + sqlx | |

### 3.2 SurrealDB feature mapping (228 features)

#### 3.2.1 Data Models (8)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-1 | Document model (JSON-like records) | ✓ | PG JSONB + tdw-domain | |
| S-2 | Graph model (first-class edges via RELATE) | ✗ | `tdw-graph` | **P0** flagship. |
| S-3 | Relational model | ✓ | PG | |
| S-4 | Key-value model | ✓ | Redis + PG primary keys | |
| S-5 | Time-series model | ✓ | CH | |
| S-6 | Geospatial model | ✗ | `tdw-spatial` (PostGIS wrapper) | P1. |
| S-7 | Vector model | ✓ | Qdrant + CH | |
| S-8 | Multi-model in one query | ✗ | `tdw-runtime` cross-engine planner | P2 (significant). |

#### 3.2.2 Record IDs (11)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-9 | Composite `table:id` form | ◐ | convention | Document in style guide. |
| S-10 | Random IDs (20-char alphanumeric) | ✓ | nanoid crate | |
| S-11 | ULID IDs | ✓ | `ulid` crate | |
| S-12 | UUID v7 IDs | ✓ | `uuid` crate | |
| S-13 | Numeric IDs (int64) | ✓ | PG BIGSERIAL | |
| S-14 | Text IDs | ✓ | PG TEXT PK | |
| S-15 | Array IDs (composite) | ✓ | PG composite PK | |
| S-16 | Object IDs (structured) | ◐ | PG JSONB PK with B-tree expression idx | |
| S-17 | Range IDs (`table:1..100`, `table:'a'..'z'`) | ✓ | PG range queries | |
| S-18 | Typed record fields (`record<table>`) | ✗ | `tdw-define` | P1. |
| S-19 | Dynamic ID construction (`type::record()`) | ✓ | runtime helper | |

#### 3.2.3 Schema (DEFINE statements) (20)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-20 | Schemafull mode | ✓ | PG tables + CHECK | |
| S-21 | Schemaless mode | ✓ | PG JSONB column | |
| S-22 | DEFINE FIELD with type | ✓ | PG column | |
| S-23 | `ASSERT $value …` (validation predicate) | ✗ | `tdw-define` + PG CHECK | Easy via CHECK constraints. |
| S-24 | `VALUE $value …` (computed/transformed) | ✗ | `tdw-define` + PG GENERATED | |
| S-25 | `DEFAULT …` | ✓ | PG DEFAULT | |
| S-26 | `READONLY` (immutable after create) | ✗ | `tdw-define` + PG trigger | |
| S-27 | `FLEXIBLE` (free-form nested) | ✓ | JSONB column | |
| S-28 | `DEFINE TABLE AS SELECT` (materialised view) | ✓ | CH MV + dbt | |
| S-29 | `DEFINE TABLE TYPE RELATION FROM x TO y` (edge table) | ✗ | `tdw-graph` | P0. |
| S-30 | `DEFINE NAMESPACE / DATABASE` | ✓ | PG schemas | |
| S-31 | `DEFINE PARAM` (persisted global params) | ✗ | `tdw-define` | P1; new table `system.params`. |
| S-32 | `DEFINE SEQUENCE` | ✓ | PG SEQUENCE | |
| S-33 | `DEFINE CONFIG` (DB-wide config) | ✗ | `tdw-define` | P1. |
| S-34 | `DEFINE BUCKET` (blob/file storage) | ✗ | `tdw-stage` | Covers via stages. |
| S-35 | `DEFINE MODULE / DEFINE API` (REST endpoints) | ✗ | `tdw-service` extension | P2. |
| S-36 | `REMOVE` / `ALTER` / `REBUILD INDEX` | ✓ | PG DDL | |
| S-37 | `INFO FOR DB/NS/TABLE/USER` | ✓ | PG information_schema | |
| S-38 | Modifiers `IF NOT EXISTS`, `OVERWRITE`, `CONCURRENTLY`, `DEFER` | ✓ | PG | |
| S-39 | Schema DDL as idempotent migrations | ✓ | sqlx-migrate + tdw-define | |

#### 3.2.4 Query Language Surface (24)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-40 | CRUD (SELECT/INSERT/UPDATE/DELETE/CREATE/UPSERT) | ✓ | PG + CH | |
| S-41 | `MERGE / PATCH / CONTENT / SET` write modes | ◐ | PG ON CONFLICT + JSONB ops | Surface in `tdw-runtime` API. |
| S-42 | `RELATE` (create edge) | ✗ | `tdw-graph` | P0. |
| S-43 | `LIVE SELECT` / `KILL` (subscriptions) | ✗ | `tdw-live` | **P0**. |
| S-44 | Transactions (BEGIN/COMMIT/CANCEL) | ✓ | PG | |
| S-45 | Flow control (IF/FOR/BREAK/CONTINUE/RETURN/THROW/SLEEP) | ✓ | PG plpgsql | |
| S-46 | `LET $x = …` | ✓ | PG psql variables | |
| S-47 | `USE NS / DB` | ✓ | PG `SET search_path` | |
| S-48 | `INFO FOR …` | ✓ | information_schema | |
| S-49 | `SHOW CHANGES FOR TABLE` | ✗ | `tdw-stream` | Covered by streams. |
| S-50 | `EXPLAIN [FULL]` | ✓ | both | |
| S-51 | Parameterized queries (`$param`) | ✓ | sqlx + tonic | |
| S-52 | `RETURN BEFORE | AFTER | DIFF | NONE | VALUE expr` | ✗ | `tdw-runtime` | Wrap RETURNING. |
| S-53 | `ONLY` (single-record return) | ✓ | runtime helper | |
| S-54 | `PARALLEL` | ✓ | CH parallel | |
| S-55 | `TIMEOUT 5s` (per-statement) | ✓ | PG `statement_timeout` | |
| S-56 | `WITH INDEX <name>` hint | ✓ | PG hints + CH | |
| S-57 | `OMIT field` (exclude from SELECT) | ✓ | CH `SELECT * EXCEPT` | PG via explicit columns. |
| S-58 | Idiom paths (`obj.field[*].nested`) | ✓ | PG JSONB ops | |
| S-59 | Casts | ✓ | both | |
| S-60 | Comments | ✓ | both | |
| S-61 | Formatters | ✓ | sqlfluff in dev | |
| S-62 | `LET` scoped within query | ✓ | PG WITH | |
| S-63 | Closures (first-class functions) | — | — | Not idiomatic in PG/CH; skip. |

#### 3.2.5 Graph Features (11)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-64 | Edge records (`RELATE in -> table -> out CONTENT {…}`) | ✗ | `tdw-graph` | P0. |
| S-65 | `->` outgoing traversal | ✗ | `tdw-graph` | Macro / fn over recursive CTE. |
| S-66 | `<-` incoming traversal | ✗ | `tdw-graph` | |
| S-67 | `<->` bidirectional traversal | ✗ | `tdw-graph` | |
| S-68 | Edge filtering inline (`->knows[WHERE since > …]->`) | ✗ | `tdw-graph` | |
| S-69 | Recursive depth (`.{N}`, `.{N..M}`, `.{..}`) | ✗ | `tdw-graph` | PG WITH RECURSIVE. |
| S-70 | `@.` and `@@` recursion references | ✗ | `tdw-graph` | |
| S-71 | Anonymous edge tables | ◐ | `tdw-graph` | Explicit table required in PG. |
| S-72 | FETCH for record links (eager-load) | ✗ | `tdw-runtime` | Resolve refs at fetch time. |
| S-73 | Reverse references (`<->refs[…]`) | ✗ | `tdw-graph` | |
| S-74 | `DEFINE TABLE TYPE RELATION FROM x TO y` | ✗ | `tdw-graph` + `tdw-define` | |

Implementation note: backed by Postgres + Apache AGE (Cypher-on-PG) OR custom recursive-CTE library. ADR-0015 picks one.

#### 3.2.6 Live Queries (7)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-75 | `LIVE SELECT` (subscribe) | ✗ | `tdw-live` | **P0**. |
| S-76 | `KILL $uuid` (unsubscribe) | ✗ | `tdw-live` | |
| S-77 | Per-event delivery (CREATE/UPDATE/DELETE) | ✗ | `tdw-live` | Via PG LISTEN/NOTIFY + logical replication. |
| S-78 | `DIFF` mode (JSON-patch diffs) | ✗ | `tdw-live` | `json-patch` crate. |
| S-79 | WebSocket transport | ◐ | `tdw-service` | Add WS endpoint. |
| S-80 | WHERE filtering on live | ✗ | `tdw-live` | Server-side predicate. |
| S-81 | Permissions-aware live | ✗ | `tdw-auth` + `tdw-live` | PG RLS in live query path. |

#### 3.2.7 Indexes (9)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-82 | Standard non-unique | ✓ | PG B-tree | |
| S-83 | Unique | ✓ | PG UNIQUE | |
| S-84 | Composite / multi-field | ✓ | PG | |
| S-85 | Expression indexes | ✓ | PG functional index | |
| S-86 | Full-text BM25 | ✓ | Meilisearch | |
| S-87 | HNSW vector | ✓ | Qdrant | |
| S-88 | MTREE vector (exact KNN) | ◐ | Qdrant exact | |
| S-89 | DISKANN disk-based ANN | ◐ | Qdrant on-disk | |
| S-90 | `CONCURRENTLY` (non-blocking build) | ✓ | PG | |
| S-91 | `REBUILD INDEX` | ✓ | PG REINDEX | |

#### 3.2.8 Built-in Function Modules (~120 individual functions)

All `module::fn` families enumerated by the research agent. Status assessment per family:

| Family | # of fns | Status | Coverage notes |
|--------|----------|--------|----------------|
| `array::` | ~50 | ✓ | PG + CH array funcs |
| `string::` | ~40 (incl. distance, html, is::, semver, similarity) | ◐ | core ✓; `is::*` predicates and semver gap → `tdw-fn-string` PG extension |
| `time::` | ~30 | ✓ | PG + CH |
| `duration::` | ~10 | ✓ | PG INTERVAL |
| `math::` | ~40 + constants | ✓ | both |
| `vector::` | ~20 (incl. distance, similarity) | ✓ | Qdrant + CH |
| `crypto::` (blake3/argon2/bcrypt/pbkdf2/scrypt) | ~15 | ◐ | PG `pgcrypto` covers most; blake3 gap |
| `geo::` | ~10 | ◐ | PostGIS |
| `http::` (server-side HTTP client) | ~6 | ✗ | UDF in `tdw-udf-external` |
| `parse::` (email, url) | ~8 | ✓ | PG ext + CH funcs |
| `type::` (casts, predicates) | ~15 | ✓ | PG + CH |
| `rand::` | ~10 | ✓ | both |
| `encoding::` (base64, hex, cbor) | ~6 | ✓ | both |
| `object::` (entries, keys, values, from_entries) | ~5 | ✓ | PG JSONB |
| `session::` (db, id, ip, ns, sc, sd, token) | ~9 | ✗ | `tdw-auth` |
| `meta::` (id, tb) | 2 | ✓ | runtime |
| `record::` (exists, id, tb, refs) | 4 | ✓ | `tdw-define` |
| `search::` (score, highlight, offsets, analyze) | 4 | ✓ | Meilisearch |
| `sequence::` (nextval, currval) | 2 | ✓ | PG |
| `set::` (arithmetic) | ~6 | ✓ | PG array ops |
| `bytes::` | ~5 | ✓ | PG bytea |
| `file::` (bucket ops) | ~6 | ✗ | `tdw-stage` |
| `value::` | ~3 | ✓ | runtime |
| `api::` (DEFINE API helpers) | ~5 | — | OOS in v0.1 |
| `sleep`, `count`, `not` | 3 | ✓ | both |
| User-defined functions (DEFINE FUNCTION) | — | ✓ | PG CREATE FUNCTION |
| JS scripted functions (QuickJS embedded) | — | ✗ | `tdw-udf-js` |
| SurrealML callable models | — | ✗ | `tdw-eval-runner` extension (Phase 8 covers part) |

Aggregate: ~120 functions, of which ~85 are direct PG/CH equivalents (✓), ~25 partial (◐), ~10 gaps (✗ — primarily `http::`, `session::`, `file::`, JS scripted UDFs, SurrealML).

#### 3.2.9 Events / Triggers (6)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-92 | `DEFINE EVENT … WHEN … THEN { … }` | ✗ | `tdw-define` + PG triggers | Declarative wrapper. |
| S-93 | Lifecycle events (CREATE/UPDATE/DELETE) | ✓ | PG trigger | |
| S-94 | `$before` / `$after` snapshots | ✓ | PG OLD/NEW | |
| S-95 | `$value` row reference | ✓ | PG NEW | |
| S-96 | Async event execution | ✗ | `tdw-notify` + RiverQueue | |
| S-97 | Cascading writes (event → another write) | ✓ | PG trigger chains | |

#### 3.2.10 Authentication (12)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-98 | System users (ROOT/NS/DB level) | ✓ | PG roles | |
| S-99 | Record users (per-row auth) | ✗ | `tdw-auth` + PG RLS | P1. |
| S-100 | JWT access (HS/RS/ES/PS/EdDSA) | ✗ | `tdw-auth` | P0. |
| S-101 | JWKS URL (dynamic key fetch) | ✗ | `tdw-auth` | |
| S-102 | Bearer grants (API-key with revocation) | ✗ | `tdw-auth` | |
| S-103 | DEFINE SCOPE / DEFINE TOKEN (legacy) | — | — | Skip; use modern. |
| S-104 | `SIGNIN { … }` / `SIGNUP { … }` | ✗ | `tdw-auth` | |
| S-105 | `AUTHENTICATE` clause (server validation) | ✗ | `tdw-auth` | |
| S-106 | `DURATION FOR TOKEN/SESSION …` | ✗ | `tdw-auth` | |
| S-107 | `$auth`, `$session`, `$token` | ✗ | `tdw-auth` | |
| S-108 | OAuth / OIDC | ✗ | `tdw-auth` | P1. |
| S-109 | Multi-issuer / multi-tenant tokens | ◐ | `tdw-auth` | P2. |

#### 3.2.11 Permissions (8)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-110 | Table-level PERMISSIONS (per CRUD op) | ✓ | PG GRANT | |
| S-111 | Per-operation predicates (select/create/update/delete) | ✓ | PG RLS | |
| S-112 | Field-level PERMISSIONS | ✗ | `tdw-mask` | Column GRANT + masking layer. |
| S-113 | Record-level predicates (referencing `$auth` etc.) | ✗ | `tdw-auth` + RLS | |
| S-114 | `PERMISSIONS NONE` / `FULL` | ✓ | PG REVOKE/GRANT | |
| S-115 | RBAC built-in (OWNER/EDITOR/VIEWER) | ✓ | PG roles | Seed default roles. |
| S-116 | Namespace/DB scoping | ✓ | PG schemas | |
| S-117 | `IF`-guarded queries | ✓ | runtime conditional | |

#### 3.2.12 Geo / Spatial (10)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-118 | Geometry literals (point, line, polygon, multi*, collection) | ✗ | `tdw-spatial` | P1; PostGIS native. |
| S-119 | GeoJSON encoding | ✓ | PostGIS `ST_AsGeoJSON` | |
| S-120 | INSIDE/OUTSIDE/INTERSECTS/CONTAINS/CONTAINSALL/ALLINSIDE | ✗ | `tdw-spatial` | ST_* funcs. |
| S-121 | Spatial indexes (GIST) | ✓ | PostGIS | |
| S-122 | `geo::distance` (haversine) | ✗ | `tdw-spatial` | ST_DistanceSphere. |
| S-123 | `geo::area` | ✗ | `tdw-spatial` | ST_Area. |
| S-124 | `geo::bearing` | ✗ | `tdw-spatial` | ST_Azimuth derivative. |
| S-125 | `geo::centroid` | ✗ | `tdw-spatial` | ST_Centroid. |
| S-126 | `geo::hash::encode/decode` (geohash) | ✗ | `tdw-spatial` | `geohash` crate. |
| S-127 | H3 hex-grid funcs | ✗ | `tdw-spatial` | `h3-rs` crate. |

#### 3.2.13 Vector / ML (8)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-128 | Vector storage (`array<float, N>`) | ✓ | Qdrant + CH | |
| S-129 | HNSW ANN | ✓ | Qdrant | |
| S-130 | MTREE exact KNN | ◐ | Qdrant exact | |
| S-131 | DISKANN | ◐ | Qdrant on-disk | |
| S-132 | `<|N,M|>` KNN operator | ✗ | `tdw-runtime` macro | Sugar over Qdrant call. |
| S-133 | Hybrid search (BM25 + vector RRF) | ✓ | already in Phase 4 | |
| S-134 | SurrealML (train PyTorch/TF/Sklearn → ONNX) | ✗ | `tdw-ml-registry` | P2. |
| S-135 | RAG patterns (vector → graph → fetch in one query) | ◐ | runtime composition | P1; needs `tdw-graph`. |

#### 3.2.14 Transactions (7)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-136 | ACID multi-statement (BEGIN…COMMIT) | ✓ | PG | |
| S-137 | Implicit per-statement | ✓ | PG | |
| S-138 | Snapshot isolation (MVCC) | ✓ | PG | |
| S-139 | `CANCEL TRANSACTION` (rollback) | ✓ | PG | |
| S-140 | `THROW` inside tx | ✓ | PG RAISE EXCEPTION | |
| S-141 | Atomic UPSERT | ✓ | PG ON CONFLICT | |
| S-142 | Optimistic concurrency (MVCC) | ✓ | PG | |

#### 3.2.15 Storage Backends (6)

| # | Feature | Status | Notes |
|---|---------|--------|-------|
| S-143 | In-memory (Mem) | ◐ | CH Memory engine. |
| S-144 | SurrealKV (versioned KV) | — | OOS. |
| S-145 | RocksDB | — | OOS. |
| S-146 | TiKV (distributed) | — | OOS. |
| S-147 | FoundationDB | — | OOS. |
| S-148 | IndexedDB (browser) | — | OOS. |

#### 3.2.16 Connectivity (8)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-149 | HTTP REST | ✓ | tdw-service | |
| S-150 | WebSocket RPC | ✗ | tdw-service + tdw-live | P0. |
| S-151 | CBOR wire format | ◐ | `ciborium` crate optional | |
| S-152 | JSON wire format | ✓ | serde_json | |
| S-153 | Embedded Rust crate | ✓ | tdw-core direct embed | |
| S-154 | WASM build | — | — | OOS. |
| S-155 | GraphQL endpoint | ✗ | tdw-service ext | P2. |
| S-156 | DEFINE API (custom REST endpoints) | ✗ | tdw-service ext | P2. |
| S-157 | MCP server | ✓ | tdw-mcp | |
| S-158 | SDKs (Rust/JS/Python/Go/Java/.NET/PHP) | ◐ | PG drivers + HTTP | Most via PG wire. |

#### 3.2.17 Data Types (17)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-159 | string / int / float / decimal / number / bool | ✓ | PG + CH | |
| S-160 | datetime (RFC 3339 + timezone) | ✓ | PG TIMESTAMPTZ | |
| S-161 | duration (`1y2w3d4h5m6s…`) | ✓ | PG INTERVAL | |
| S-162 | bytes | ✓ | PG bytea | |
| S-163 | uuid v4 / v7 | ✓ | PG uuid | |
| S-164 | record<table> typed link | ✗ | `tdw-define` | P1. |
| S-165 | geometry (GeoJSON union) | ✗ | `tdw-spatial` | P1. |
| S-166 | array<T, N> typed + length-constrained | ✓ | PG array + CHECK | |
| S-167 | set<T> (auto-dedup ordered) | ◐ | PG array + UNIQUE constraint | |
| S-168 | object (nested map) | ✓ | PG JSONB | |
| S-169 | range (`1..10`) | ✓ | PG int4range, daterange | |
| S-170 | regex (`/…/`) | ✓ | PG regex | |
| S-171 | literal (enum-like union `'a' | 'b' | int`) | ✓ | PG enum + check | |
| S-172 | option<T> | ✓ | PG nullable | |
| S-173 | any (wildcard) | ✓ | PG JSONB | |
| S-174 | none vs null distinct | ◐ | PG NULL + JSON null | |
| S-175 | future (lazy values) | — | — | Skip; unusual. |
| S-176 | closure (first-class fn values) | — | — | Skip. |
| S-177 | file (bucket-backed ref) | ✓ | tdw-stage BlobRef | |

#### 3.2.18 Query Features (15)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-178 | Subqueries (anywhere) | ✓ | PG + CH | |
| S-179 | GROUP BY / GROUP ALL | ✓ | both | |
| S-180 | ORDER BY (multi-key, ASC/DESC, COLLATE, NUMERIC, RAND) | ✓ | both | |
| S-181 | LIMIT / START (offset) | ✓ | both | |
| S-182 | FETCH (eager-resolve links) | ✗ | `tdw-runtime` | Helper for joined fetches. |
| S-183 | SPLIT (unnest arrays) | ✓ | PG unnest + CH arrayJoin | |
| S-184 | VALUE (flatten single column) | ✓ | runtime helper | |
| S-185 | PARALLEL | ✓ | CH | |
| S-186 | EXPLAIN [FULL] | ✓ | both | |
| S-187 | WITH INDEX / WITH NOINDEX hints | ✓ | PG hints | |
| S-188 | TIMEOUT | ✓ | PG | |
| S-189 | ONLY (single-result) | ✓ | runtime | |
| S-190 | OMIT (drop fields) | ✓ | CH; PG explicit | |
| S-191 | Idiom paths (`obj.field[*].nested`) | ✓ | JSONB ops | |
| S-192 | Destructuring (`SELECT {a, b}`) | ◐ | runtime sugar | |

#### 3.2.19 Embedded Mode (5)

| # | Feature | Status |
|---|---------|--------|
| S-193 | In-process Rust | ✓ via tdw-core direct embed |
| S-194 | Browser WASM | — OOS |
| S-195 | Node.js native | — OOS |
| S-196 | Python embedded | ◐ via PyO3 wrapper (P2) |
| S-197 | Mobile | — OOS |

#### 3.2.20 Replication / Clustering (6)

All — OOS.

#### 3.2.21 Migrations (7)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-198 | DEFINE statements as idempotent DDL | ✓ | sqlx-migrate | |
| S-199 | `IF NOT EXISTS` / `OVERWRITE` | ✓ | PG | |
| S-200 | ALTER statements | ✓ | PG | |
| S-201 | REMOVE statements | ✓ | PG DROP | |
| S-202 | REBUILD INDEX | ✓ | PG REINDEX | |
| S-203 | `DEFINE TABLE … CHANGEFEED 3d INCLUDE ORIGINAL` | ✗ | `tdw-stream` | Same as Databend streams. |
| S-204 | Migration tool (no native; community add-on) | ✓ | sqlx-migrate + tdw-migration | |

#### 3.2.22 Notebook / App Integration (8)

| # | Feature | Status | Notes |
|---|---------|--------|-------|
| S-205 | Surrealist UI | — | OOS. |
| S-206 | GraphQL endpoint | ✗ | P2. |
| S-207 | REST KV API | ✓ | tdw-service. |
| S-208 | REST SQL API | ✓ | tdw-service. |
| S-209 | OpenTelemetry | ✓ | tdw-runtime tracing. |
| S-210 | Docker / Helm / K8s | ◐ | Docker ✓; Helm/K8s P2. |
| S-211 | MCP server | ✓ | tdw-mcp. |
| S-212 | LangChain integration | ◐ | consumer-side, document. |

#### 3.2.23 Distinctive Capabilities (16)

| # | Feature | Status | Owner | Notes |
|---|---------|--------|-------|-------|
| S-213 | Computed/materialised views auto-updated on writes | ✓ | CH MV | |
| S-214 | Change feeds (`CHANGEFEED 3d INCLUDE ORIGINAL`) | ✗ | `tdw-stream` | P0. |
| S-215 | `SHOW CHANGES FOR TABLE` since versionstamp | ✗ | `tdw-stream` | |
| S-216 | Futures (deferred lazy values) | — | — | OOS. |
| S-217 | Embedded JS runtime (QuickJS) | ✗ | `tdw-udf-js` | P1. |
| S-218 | Server-side HTTP client (`http::*`) | ✗ | `tdw-udf-external` | |
| S-219 | DEFINE API (REST endpoints in SQL) | ✗ | `tdw-service` ext | P2. |
| S-220 | DEFINE BUCKET (blob abstraction) | ✗ | `tdw-stage` | Covers. |
| S-221 | DEFINE SEQUENCE | ✓ | PG | |
| S-222 | DEFINE PARAM (global params) | ✗ | `tdw-define` | |
| S-223 | DEFINE CONFIG | ✗ | `tdw-define` | |
| S-224 | Multi-tenancy (NS/DB primitives) | — | — | OOS. |
| S-225 | OpenAPI / GraphQL gen | ✗ | tdw-service ext | P2. |
| S-226 | SurrealML model registry + inference | ✗ | `tdw-ml-registry` | P2. |
| S-227 | `record::refs()` reverse-link resolution | ✗ | `tdw-graph` | |
| S-228 | PERMISSIONS on computed (view) tables | ✓ | PG RLS works on views | |

---

## 4. Gap summary (what's actually new work)

Tallying the mapping tables — only counting features with status ✗ (Gap):

| Cluster | Count | Phase | Crate(s) |
|---------|-------|-------|----------|
| **Time travel + snapshots** (D-1, 2, 6, 9, 49–59, 143, 144, 151, 178, 180, 187, 188, 247) | 18 | Phase 9 | `tdw-snapshot` |
| **Streams / CDC** (D-60–69, S-49, 203, 214, 215) | 14 | Phase 10 | `tdw-stream` |
| **Live queries** (S-43, 75–81, 150) | 9 | Phase 10 | `tdw-live` |
| **Graph layer** (S-2, 29, 42, 64–74, 227) | 14 | Phase 11 | `tdw-graph` |
| **Geometry/spatial** (S-6, 118, 120, 122–127, 165) | 10 | Phase 11 | `tdw-spatial` |
| **Stages + COPY INTO** (D-7, 14, 15, 80–91, 145, 156, 249, S-34, 220) | 18 | Phase 12 | `tdw-stage` |
| **Open table formats** (D-148, 149, 152–154) | 5 | Phase 12 | `tdw-table-format` |
| **Pipes (auto-ingest)** (D-252) | 1 | Phase 12 | `tdw-pipe` (sub-module) |
| **UDFs (Python/JS/WASM/external/sandbox)** (D-99–102, 104, 105, 248, 250, S-217, 218, function families ◐/✗) | 12 | Phase 13 | `tdw-udf`, `tdw-udf-python`, `tdw-udf-js`, `tdw-udf-wasm`, `tdw-udf-external` |
| **Auth (JWT/scopes/sessions/tokens)** (D-119, S-99–109, function fam `session::`) | 13 | Phase 13 | `tdw-auth` |
| **Define-style declarative schema** (S-18, 23, 24, 26, 31, 33, 36, 38, 92, 164, 222, 223, D-36 multi-insert + ALTER MV) | 14 | Phase 13 | `tdw-define` |
| **Tags + governance metadata** (D-256) | 1 | Phase 9 (with snapshots) | `tdw-tags` sub-module |
| **Masking policies** (D-114, S-112) | 2 | Phase 13 | `tdw-mask` sub-module |
| **Notifications (webhook/queue hooks)** (D-73, 255, S-96) | 3 | Phase 9 (early; used by tasks) | `tdw-notify` sub-module |
| **Auto MV rewrite** (D-30, 95, 166) | 3 | Phase 13 (optional) | `tdw-rewrite` sub-module |
| **SurrealML / model registry** (S-134, 226) | 2 | Phase 8 / P2 | extends `tdw-eval-runner` |
| **GraphQL / DEFINE API** (S-155, 156, 206, 219, 225) | 5 | P2 | `tdw-service-api` (later) |

**Total gaps**: ~140 distinct features (matches the 138 in the parent table). All addressable in Phases 9–13 plus a P2 follow-on.

---

## 5. New crates

```diff
crates/
  ...
+ tdw-snapshot/                   ← snapshot tree, time travel, undrop, flashback, tags
+ tdw-stream/                     ← CREATE STREAM, APPEND_ONLY, CHANGEFEED, SHOW CHANGES
+ tdw-live/                       ← LIVE SELECT, WebSocket push, diff mode
+ tdw-graph/                      ← edges (RELATE), traversal (->, <-, <->), recursive depth
+ tdw-spatial/                    ← PostGIS wrapper + h3-rs + geohash
+ tdw-stage/                      ← stages, COPY INTO, auto schema evolution
+ tdw-table-format/               ← Iceberg + Delta Lake (read first; write P2)
+ tdw-udf/                        ← unified UDF dispatcher (Python/JS/WASM/External)
+ tdw-udf-python/                 ← PyO3 + sandboxed subprocess
+ tdw-udf-js/                     ← QuickJS via rquickjs
+ tdw-udf-wasm/                   ← wasmtime
+ tdw-udf-external/               ← HTTP / Arrow Flight UDF clients
+ tdw-auth/                       ← JWT, scopes, sessions, tokens, OIDC, SIGNIN/SIGNUP
+ tdw-define/                     ← DEFINE TABLE/FIELD/EVENT/PARAM/CONFIG → DDL codegen
```

Sub-modules (not separate crates):
- `tdw-runtime::tags` — governance metadata
- `tdw-runtime::mask` — column masking
- `tdw-runtime::notify` — webhook/queue notifications
- `tdw-runtime::rewrite` — auto MV query rewrite
- `tdw-runtime::pipe` — auto-ingest from stage on file event
- `tdw-storage-postgres::age` — graph via Apache AGE (if ADR-0015 picks it)

Updated workspace total: **~43 crates** (parent 29 + this extension 14).

---

## 6. Implementation Phases (Phase 9–13)

### Phase 9 — Snapshots, Time Travel, Tags, Notifications — days 50–58

9.1. `tdw-snapshot` core: snapshot tree data model — `Snapshot { id, parent_id, table_ref, segments[], created_at }` stored in PG `system.snapshot`; segment metadata stored alongside (min/max/null counts per column).
9.2. Write path: every commit writes a new snapshot row + updates table's `current_snapshot_id` atomically.
9.3. Read path: `tdw-runtime` resolves `AT (SNAPSHOT|TIMESTAMP|OFFSET|STREAM => …)` to a snapshot ID and routes reads through it; CH side queries `mergeTree...` with a `_snapshot_id <= X` filter or a versioned view per snapshot tag.
9.4. `UNDROP TABLE` / `UNDROP DATABASE` — soft-delete via `deleted_at` + retention; restore copies metadata back.
9.5. `FLASHBACK TABLE TO SNAPSHOT/TIMESTAMP` — overwrites `current_snapshot_id`; preserves intermediate snapshots for further undo.
9.6. `CREATE TABLE LIKE … AT (SNAPSHOT => …)` — fork: new table pointing at the same segment files but a new identity.
9.7. Snapshot tags — `CREATE TAG <name> ON <table> AT (SNAPSHOT => …)`; named references retained beyond retention.
9.8. Per-table retention — `ALTER TABLE … SET DATA_RETENTION_TIME_IN_DAYS = N`.
9.9. `VACUUM` — physically purge segments older than retention (and not tagged); CLI: `xtask vacuum`.
9.10. `tdw-tags` sub-module: governance k/v on any object; system table `system.tag`.
9.11. `tdw-notify` sub-module: notification objects (`CREATE NOTIFICATION name URL='…' KIND='webhook'`); used for task `ERROR_INTEGRATION` and event-driven pipes.
9.12. Documentation: `docs/time-travel.md`, ADR-0016 (snapshot tree shape).

**Exit criteria**: A9.1–A9.10 satisfied (see §7).

### Phase 10 — Streams (CDC) + Live Queries — days 59–66

10.1. `tdw-stream` core: `Stream { name, table_ref, mode (AppendOnly|Standard), offset_snapshot_id, last_consumed_at }` in PG `system.stream`.
10.2. `CREATE STREAM ON TABLE foo APPEND_ONLY = TRUE` — capture all `INSERT` snapshots after creation.
10.3. Standard mode — emit `INSERT/UPDATE/DELETE` with metadata columns `change$action`, `change$is_update`, `change$row_id`. Implementation uses PG logical replication or CH versionedCollapsingMergeTree depending on source table engine.
10.4. Stream reads: `SELECT * FROM STREAM(name)` returns rows changed since last consumed offset; offset advances atomically inside the consuming transaction.
10.5. `AT (SNAPSHOT|TIMESTAMP|OFFSET|STREAM => …)` start offset; stream-to-stream cloning.
10.6. `SHOW STREAMS` / `DESC STREAM` — wraps `system.stream`.
10.7. SurrealDB `CHANGEFEED` equivalent — feature flag on table that auto-creates a hidden Standard stream with N-day retention.
10.8. `tdw-live`: LIVE SELECT primitive. Backed by PG `LISTEN/NOTIFY` for INSERTs/UPDATEs, augmented with logical replication for completeness.
10.9. WebSocket endpoint in `tdw-service` — clients open WS, send `LIVE SELECT * FROM foo WHERE …`, receive `{action, before?, after, diff?}` messages.
10.10. DIFF mode — `json-patch` crate computes the diff between before/after.
10.11. KILL via subscription UUID.
10.12. Permissions-aware live — PG RLS predicate applied to every emitted event before dispatch.
10.13. Documentation: `docs/streams-and-cdc.md`, `docs/live-queries.md`, ADR-0017.

**Exit criteria**: A10.1–A10.12 satisfied.

### Phase 11 — Graph + Spatial — days 67–74

11.1. ADR-0015: graph backend decision. **Options:**
   - **A**: Postgres + Apache AGE (Cypher on PG). Pro: integrated, supports openCypher. Con: AGE is maintenance-heavy, requires PG ext.
   - **B**: Native Rust graph engine over PG tables (edge table + recursive CTE library).
   - **C**: Sidecar Neo4j community edition. Con: extra service.
   - **Recommendation**: **B** (native Rust over PG tables) — keeps stack thin; recursive CTE handles ~95% of traversal needs.

11.2. `tdw-graph` traits — `Edge { id, from: RecordRef, to: RecordRef, kind, attributes: JsonB }`; `Vertex` is any PG row with a `record_id` column.

11.3. `RELATE in_id -> kind -> out_id CONTENT {…}` — implemented as `INSERT INTO edge_<kind> (in_id, out_id, attrs)`.

11.4. Traversal helpers (Rust API + SQL macros):
   - `traverse(v, "->kind->", depth)` → recursive CTE.
   - `<-`, `<->`, edge filtering via WHERE-pushdown.
   - Depth-bounded recursive (`.{N}`, `.{N..M}`, `.{..}` capped at 256).

11.5. `DEFINE TABLE TYPE RELATION FROM x TO y` — `tdw-define` macro that creates the edge table with typed FK constraints.

11.6. `tdw-spatial`: enable PostGIS extension; expose `ST_*` functions through `tdw-runtime` typed API; add `h3-rs` for H3 grid; `geohash` crate for geohash encode/decode.

11.7. Documentation: `docs/graph-modeling.md`, `docs/spatial.md`, ADR-0015.

**Exit criteria**: A11.1–A11.10 satisfied.

### Phase 12 — Stages, COPY INTO, Open Table Formats, Pipes — days 75–84

12.1. `tdw-stage` core: stage as a typed S3 prefix + file-format config. `Stage { name, kind (User|Internal|External), uri, credentials_ref, file_format, copy_options }`.

12.2. `CREATE STAGE my_int` — internal stage at `s3://finx-finance-data/stages/{name}/`.

12.3. `CREATE STAGE my_ext URL='s3://other-bucket/path/' …` — external.

12.4. User stage `@~` — per-user prefix at `s3://finx-finance-data/users/{user}/stage/`.

12.5. File formats: Parquet (default), CSV, NDJSON, ORC, AVRO (gap close: ORC via `arrow`, AVRO via `apache-avro`).

12.6. `COPY INTO table FROM @stage` — ETL job that streams files, validates schema, writes to ClickHouse/Postgres via WriteSink. Pattern matching (PCRE2) via `regex` crate.

12.7. Auto schema evolution — if a file has a column not in the target table, `ALTER TABLE ADD COLUMN` automatically. Setting `SCHEMA_EVOLUTION = ON` per stage or COPY call.

12.8. `LIST @stage` / `REMOVE @stage` / `PRESIGN @stage/file` — wraps `BlobEngine`.

12.9. Direct query over stage: `SELECT … FROM @stage (FILE_FORMAT => 'parquet')` — pushed through CH's S3 table function.

12.10. `tdw-table-format`: Iceberg via `iceberg-rust` crate (read-only at v0.1); Delta Lake via `delta-rs` (read-only). REST catalog + AWS Glue + Hive Metastore configurable.

12.11. `tdw-pipe` sub-module: auto-ingest. Pipe definition `{ stage, target_table, schedule | event }`. Event mode listens to S3 `ObjectCreated` notifications (via SQS or polling); schedule mode polls on cron.

12.12. Documentation: `docs/stages.md`, `docs/iceberg-delta.md`, `docs/pipes.md`, ADR-0018.

**Exit criteria**: A12.1–A12.11 satisfied.

### Phase 13 — UDFs, Auth, DEFINE-style declarative schema, Masking — days 85–95

13.1. `tdw-udf` trait crate: `Udf` trait with `kind() -> UdfKind` (Sql|Python|Js|Wasm|External), `signature()`, `invoke(args) -> Result<Value>`.

13.2. `tdw-udf-python`: PyO3 wrapper with subprocess isolation; PACKAGES clause maps to a pinned virtualenv at `~/.finx-finance/udfs/{name}/`; sandboxed via process resource limits (memory, CPU, network on/off).

13.3. `tdw-udf-js`: QuickJS via `rquickjs` crate; isolated context per call; configurable memory limit.

13.4. `tdw-udf-wasm`: wasmtime runtime; supports Arrow UDF binary format; resource-limited via wasmtime fuel.

13.5. `tdw-udf-external`: HTTP and Arrow Flight UDFs. Configurable retry/timeout; signed requests when AUTH = bearer/iam.

13.6. Aggregate UDFs (UDAF) supported in Python and JS (init / accumulate / merge / finalize).

13.7. `tdw-auth` core: `Identity { user_id, scopes, expires_at, claims }`. JWT verify with HS256–HS512, RS256–RS512, ES256–ES512, PS256–PS512, EdDSA via `jsonwebtoken` crate. JWKS URL fetched + cached with TTL.

13.8. Bearer grants (API keys) — table `auth.bearer_grant`, rotation + revocation, scope assignment. CLI: `tdw-cli auth grant create --scope read:equities --ttl 30d`.

13.9. `SIGNIN { user, pass }` and `SIGNUP { ... }` endpoints — return JWT.

13.10. Record users (PG RLS predicates referencing `current_setting('app.user_id')`).

13.11. `AUTHENTICATE` clause — server-side validation expression run on every request.

13.12. `$auth`, `$session`, `$token` — session-scoped variables set on every request.

13.13. OIDC support via `tdw-auth-oidc` feature flag.

13.14. `tdw-define` core: declarative schema files (YAML or DSL). Walks definitions and emits PG/CH DDL via `tdw-sql-codegen` (extended from Phase 7).
   - DEFINE FIELD with `ASSERT`, `VALUE`, `DEFAULT`, `READONLY`, `FLEXIBLE` → PG CHECK / GENERATED / triggers.
   - DEFINE EVENT → PG trigger.
   - DEFINE PARAM → row in `system.param`.
   - DEFINE CONFIG → row in `system.config`.
   - record<table> typed FK → PG FOREIGN KEY with metadata.

13.15. `tdw-runtime::mask` sub-module: masking policies. `CREATE MASKING POLICY mask_pii AS (val) -> CASE WHEN current_role IN ('viewer','public') THEN '***' ELSE val END`. Applied per column via PG view + column GRANT.

13.16. Auto MV rewrite (`tdw-runtime::rewrite`, optional behind feature flag) — query interceptor matches against MV definitions and rewrites if cost-cheaper.

13.17. Documentation: `docs/udfs.md`, `docs/auth.md`, `docs/define-schema.md`, `docs/masking.md`, ADR-0019 (declarative DEFINE format).

**Exit criteria**: A13.1–A13.18 satisfied.

---

## 7. Acceptance Criteria

### Phase 9 — Snapshots / Time Travel / Tags / Notifications

A9.1. `tdw-snapshot::commit(table, batch)` produces a new snapshot row and updates `current_snapshot_id` atomically; verified by 1000-write stress test that no two snapshots share an ID.
A9.2. `SELECT ... FROM foo AT (SNAPSHOT => '<id>')` returns the historical state; verified for 5 different snapshot ages.
A9.3. `SELECT ... FROM foo AT (TIMESTAMP => '2026-05-21T12:00:00Z')` resolves to the snapshot active at that time.
A9.4. `UNDROP TABLE foo` restores a dropped table within retention; outside retention returns a precise error.
A9.5. `FLASHBACK TABLE foo TO SNAPSHOT '<id>'` rewinds the live pointer; preserves intermediate snapshots for further undo.
A9.6. `CREATE TABLE foo_dev LIKE foo AT (SNAPSHOT => '<id>')` produces a forked table that diverges on write.
A9.7. Tagged snapshots (`CREATE TAG eod_2026_05_21 ON foo AT (SNAPSHOT => '<id>')`) survive `VACUUM`.
A9.8. Per-table retention enforced — `xtask vacuum --dry-run` lists segments scheduled for purge.
A9.9. `tdw-tags` allows arbitrary k/v on any system object; queryable via `SELECT * FROM system.tag WHERE object_kind = 'table' AND key = 'pii'`.
A9.10. `tdw-notify` delivers a test webhook with retries + dead-letter when a fake task fails 3 times.

### Phase 10 — Streams / Live Queries

A10.1. `CREATE STREAM foo_stream ON TABLE foo APPEND_ONLY = TRUE` creates a stream row; `SELECT * FROM STREAM(foo_stream)` returns 0 rows initially.
A10.2. After 10 INSERTs, `SELECT * FROM STREAM(foo_stream)` returns 10 rows with `change$action = 'INSERT'`.
A10.3. Consuming inside a transaction advances offset only on COMMIT; ROLLBACK preserves the prior offset.
A10.4. Standard-mode stream captures UPDATE/DELETE with correct `change$action` and `change$is_update`.
A10.5. Stream-to-stream cloning: `CREATE STREAM s2 AT (STREAM => 's1')` shares offset state initially.
A10.6. SurrealDB `SHOW CHANGES FOR TABLE foo SINCE '<versionstamp>'` returns equivalent results to a versionstamp-pinned `STREAM(...)`.
A10.7. `LIVE SELECT * FROM foo WHERE symbol = 'AAPL'` over WebSocket returns matching events as they happen; verified with 100-event burst.
A10.8. `KILL '<uuid>'` terminates the subscription; subsequent inserts produce no event to that client.
A10.9. DIFF mode emits valid RFC-6902 JSON-Patch documents; verified with `json-patch` round-trip.
A10.10. Permissions-aware live: a client with role `viewer` does not receive events on rows excluded by RLS.
A10.11. WebSocket transport supports both JSON and CBOR wire formats; clients pick via `Accept` header.
A10.12. Live subscription survives a 30-second network glitch via auto-resume from last delivered seq.

### Phase 11 — Graph + Spatial

A11.1. `RELATE alice -> knows -> bob CONTENT { since: 2020 }` creates an edge; verified row count.
A11.2. Traversal `alice ->knows-> ?` returns Bob.
A11.3. Reverse traversal `bob <-knows<- ?` returns Alice.
A11.4. Bidirectional `alice <-knows-> ?` returns Bob.
A11.5. Recursive depth-bounded `alice ->knows.{1..3}-> ?` returns Alice's network up to 3 hops.
A11.6. Cycle protection: `->knows.{..}-> ?` over a known 1000-node cycle terminates within 256 steps with a precise overflow error.
A11.7. Typed edge table: `DEFINE TABLE knows TYPE RELATION FROM person TO person` rejects edges with non-`person` endpoints.
A11.8. PostGIS enabled: `SELECT ST_Distance(point_a, point_b)` returns meters for two GIS points.
A11.9. Geohash encode/decode round-trips for 100 random coordinates with ≤ 8-character precision tolerance.
A11.10. H3 grid: `h3_geo_to_h3(lat, lng, 9)` produces a 64-bit cell ID stable across runs.

### Phase 12 — Stages / Open Table Formats / Pipes

A12.1. `CREATE STAGE my_int` creates a stage; `LIST @my_int` lists files; `PUT file.csv @my_int` uploads.
A12.2. `COPY INTO foo FROM @my_int FILE_FORMAT = (TYPE = CSV) PATTERN = '.*\\.csv'` ingests matching files.
A12.3. Auto schema evolution: a CSV with a new column triggers `ALTER TABLE foo ADD COLUMN`; verified by inspecting `\\d foo` before/after.
A12.4. Direct query: `SELECT count(*) FROM @my_int (FILE_FORMAT => 'parquet')` returns row count without ingesting.
A12.5. Iceberg read: `CREATE CATALOG iceberg_demo TYPE = ICEBERG ...; SELECT * FROM iceberg_demo.db.tbl LIMIT 10` returns rows.
A12.6. Iceberg time travel: `SELECT ... FROM iceberg_demo.db.tbl FOR SYSTEM_VERSION AS OF <snapshot>` works.
A12.7. Delta Lake read: equivalent path via `delta-rs`.
A12.8. `tdw-pipe` polling mode: a new file in a watched stage triggers a COPY within 60s of arrival.
A12.9. `tdw-pipe` event mode (with SQS-style notifications): new file triggers COPY within 5s.
A12.10. ORC + AVRO formats: round-trip a sample dataset; checksums match.
A12.11. `REMOVE @my_int/old.csv` deletes the object from S3; verified via `aws s3 ls`.

### Phase 13 — UDFs / Auth / DEFINE / Masking

A13.1. Python UDF: `CREATE FUNCTION my_udf(x INT) RETURNS INT LANGUAGE python AS $$ return x * 2 $$` and `SELECT my_udf(5)` returns 10.
A13.2. JS UDF: same but `LANGUAGE javascript`; isolated context; verified that `globalThis.foo` does not leak between calls.
A13.3. WASM UDF: load a precompiled wasm module via `LANGUAGE wasm IMPORTS('mod.wasm')`; invoke; resource limits enforced.
A13.4. Python UDF with PACKAGES: `PACKAGES = ['numpy==2.0']` creates a sandboxed venv; numpy operations work.
A13.5. External Function: HTTP UDF `LANGUAGE external URL='http://localhost:9100/predict'` round-trips; configurable retries verified.
A13.6. Sandboxed UDF: a Python UDF that tries to `open('/etc/passwd')` is denied by sandbox policy.
A13.7. JWT auth: `tdw-cli auth signin --user alice` returns a JWT; subsequent requests with `Authorization: Bearer <jwt>` succeed.
A13.8. JWKS: configure issuer URL; rotation tested by re-fetching keys after cache TTL.
A13.9. Bearer grant: `tdw-cli auth grant create --scope read:equities --ttl 30d` creates a token; revocation removes access.
A13.10. SIGNUP/SIGNIN endpoints round-trip a user creation flow.
A13.11. `$auth.user_id` available inside a query and matches the bearer token's `sub` claim.
A13.12. AUTHENTICATE clause: a custom `AUTHENTICATE { … }` block runs on every request and can deny based on time-of-day.
A13.13. Record user with PG RLS: a user with `record_id = 'user:alice'` only sees rows where `owner = 'user:alice'`.
A13.14. `tdw-define` ingests a YAML schema (10 tables, 50 fields, 10 events) and emits idempotent PG + CH DDL; running it twice produces zero diffs.
A13.15. ASSERT predicate enforced: insert violating ASSERT is rejected with a clear error.
A13.16. VALUE field transformation applied on write.
A13.17. READONLY field rejects UPDATE.
A13.18. Masking policy: a column tagged `MASKING POLICY mask_pii` returns `***` to role `viewer` and the real value to `editor`.

---

## 8. Risks & Mitigations

| #    | Risk | Likelihood | Impact | Mitigation |
|------|------|-----------|--------|------------|
| R26  | Snapshot tree write overhead degrades hot ingest path | High | High | Snapshot writes batched per-commit (not per-row); benchmark in Phase 9 against current ingest path; abort if >15% degradation and switch to coarser snapshot granularity (per-partition rather than per-commit). |
| R27  | Stream offset advancement under concurrent consumers causes lost events | Medium | High | Use PG advisory locks per stream consumer; offset advances only inside the consumer's COMMIT; double-consumption protection via consumer-group concept. |
| R28  | Live queries fan-out at high write rate (>10k events/sec) overwhelms WS clients | High | Medium | Per-subscription rate-limit + drop-with-warning when client falls behind by >5k events; deliver "MISSED N events" sentinel; client must re-snapshot. |
| R29  | Graph layer recursive CTE explodes under cyclic traversal | High | High | Hard depth cap (256); per-traversal node-visit budget (max 1M); abort with `GraphTraversalLimitExceeded` precise error. |
| R30  | PostGIS extension increases Postgres footprint + complicates migrations | Medium | Low | Optional via `--profile spatial` in docker-compose; only enabled in `tdw-storage-postgres` when feature `spatial` is on. |
| R31  | Iceberg + Delta both have evolving Rust ecosystems; APIs may break | Medium | Medium | Pin minor versions; abstraction layer in `tdw-table-format` insulates callers; read-only at v0.1 means no commit-side complexity. |
| R32  | UDF sandboxes leak resources / get exploited by malicious agent code | Medium | **High** | Process isolation + wasmtime fuel + Python venv + JS heap limits; deny-by-default network policy; periodic security audit; integration tests with adversarial UDFs. |
| R33  | JWT key rotation breaks active sessions | Medium | Medium | Grace period (configurable, default 24h) where old keys still validate; `tdw-cli auth keys rotate` documents the procedure. |
| R34  | Auto schema evolution on COPY adds the wrong column type | Medium | Medium | Default to widening type rules (INT → BIGINT, FLOAT → DOUBLE, anything → TEXT); explicit `--strict-schema` mode rejects unknown columns; logged + reviewable. |
| R35  | DEFINE-style schema becomes a third source of truth (alongside Rust structs + dbt sources) | High | High | DEFINE → Rust struct codegen is one-way; CI gate verifies DEFINE files generate the same Rust types currently in `tdw-domain`. |
| R36  | Masking policies bypassed via direct SQL access | Medium | High | Direct PG access only granted to admin role; all app/MCP/HTTP paths go through `tdw-runtime` which enforces masking; CI test verifies no path skips. |
| R37  | Apache AGE / native graph backend choice (ADR-0015) regretted | Low | Medium | Implementation hidden behind `tdw-graph` trait; can swap backend without changing call sites. |
| R38  | Phase 9–13 estimated at 46 days; total project becomes ~97 days | — | High | Acknowledged; Phase 9 + 10 cover ~60% of the value; defer 11/12/13 to v0.2 if needed. |
| R39  | Stream + LiveQuery + Snapshot interactions create complex consistency edge cases | High | High | Property-based testing via `proptest`; explicit semantics doc per pair; CI fuzz harness running 24h before each release. |

---

## 9. Verification Steps

V33. `cargo test -p tdw-snapshot --features integration` — runs the 1000-write stress + flashback + undrop scenarios. (A9.1–A9.5)
V34. `cargo bench -p tdw-snapshot` measures snapshot-write overhead vs no-snapshot baseline; CI gate at <15% regression. (R26)
V35. `cargo test -p tdw-stream --features integration` — stream consume + offset + rollback semantics. (A10.1–A10.6)
V36. `cargo test -p tdw-live --features integration` — WebSocket subscribe + filter + DIFF + RLS + auto-resume. (A10.7–A10.12)
V37. `proptest` fuzz: random sequences of `insert/update/delete/commit/rollback/stream-read` over 1h produce no inconsistency. (R27, R39)
V38. `cargo test -p tdw-graph` — RELATE + traversal + recursive bounds + cycle protection + typed RELATION. (A11.1–A11.7)
V39. `cargo test -p tdw-spatial --features postgis` — PostGIS distance + H3 + geohash. (A11.8–A11.10)
V40. `cargo test -p tdw-stage --features integration` — stage create/list/put/get/remove + COPY INTO + schema evolution + pattern matching. (A12.1–A12.3, A12.11)
V41. `cargo test -p tdw-table-format --features integration` — Iceberg + Delta read against testcontainers MinIO + Iceberg REST catalog. (A12.5–A12.7)
V42. `cargo test -p tdw-pipe` — auto-ingest in polling + event modes. (A12.8, A12.9)
V43. UDF security suite — adversarial Python/JS/WASM/External UDFs attempt sandbox escape; all fail. (R32)
V44. `cargo test -p tdw-auth --features integration` — JWT, JWKS, bearer grants, OIDC. (A13.7–A13.13)
V45. `cargo test -p tdw-define` — YAML → Rust struct codegen byte-stable; second run is zero-diff. (A13.14, R35)
V46. `cargo test -p tdw-runtime --test masking` — masking policy verified per role. (A13.18, R36)
V47. **Feature parity matrix audit** — `xtask parity-check` walks `docs/parity/databend.md` and `docs/parity/surrealdb.md`, verifies every ✓/◐ row has a passing test. CI gate. (this whole plan)

---

## 10. ADR — Architecture Decision Record

- **Decision**: Replicate Databend's + SurrealDB's feature surface (~440 features) on top of the existing FinX-Finance specialist stack via 14 new crates and 5 phases (9–13), without adopting either system as a runtime.

- **Drivers**:
  1. The parent plan's SKIP verdicts on Databend and SurrealDB were correct *as runtimes* (license, scope, specialist-vs-generalist), but their **feature surfaces** are valuable and should be present in FinX-Finance.
  2. Personal codebase: no operational burden of running their servers; we want the capabilities baked into ours.
  3. Clean-room boundary preserved — we read docs, not source.

- **Alternatives considered**:
  - **Adopt Databend as the OLAP layer** (reverse parent plan's SKIP): rejected — license, lock-in, specialist-vs-generalist principle violation.
  - **Adopt SurrealDB embedded** for graph + live queries: rejected — same reason; also BSL 1.1 license trap.
  - **Pick a subset of features** (skip ~half of the ✗ items): viable; tracked as "v0.1 vs v0.2" within each phase.
  - **Build a new query language (SurrealQL/Databend SQL clone)**: rejected — multi-year effort with no payoff for personal use; specialist SQL surfaces (PG + CH) suffice.

- **Why chosen**: All targeted capabilities are valuable to the user (time travel for audit, streams for CDC, live queries for dashboards, graph for relationship modeling, UDFs for in-DB ML, auth for hosting), and each maps cleanly onto an existing engine in the stack. No single feature requires a new query language or new runtime.

- **Consequences**:
  - +14 crates (43 total).
  - +46 days (Phases 9–13); parallelizable to ~30 days if 9+10 are paired with 11+12.
  - Snapshot tree adds write-path overhead (R26); benchmarks gate the design.
  - DEFINE-style declarative schema introduces a third source of truth alongside Rust structs and dbt sources; CI gate (R35) keeps them aligned.
  - PostGIS adds Postgres footprint; opt-in feature flag.
  - UDF sandboxes are a non-trivial security surface (R32); explicit adversarial-test harness.

- **Follow-ups**:
  - ADR-0015 — graph backend choice (Apache AGE vs native Rust over PG; recommendation: native).
  - ADR-0016 — snapshot tree shape (per-commit vs per-partition snapshot granularity).
  - ADR-0017 — stream + live-query consistency model.
  - ADR-0018 — stage credential resolution (named connections vs inline vs IAM role).
  - ADR-0019 — DEFINE-style schema DSL choice (YAML vs Pkl vs custom DSL).
  - O11 — should we ship Iceberg write support in v0.2 (currently read-only at v0.1)?
  - O12 — Apache AGE vs native graph engine choice (deferred to ADR-0015).
  - O13 — SurrealML / model registry — does this belong in `tdw-eval-runner` (Phase 8) or its own crate?
  - O14 — auto MV rewrite (D-30) — ship or defer?
  - O15 — multi-model unified query layer (S-8) — design now or punt to v0.2?

---

## 11. Combined timeline

| Phase | Description | Days | Cumulative |
|-------|-------------|------|------------|
| 0.0   | Discovery (BOM re-derive, ADRs, license) | 0–2 | 2 |
| 0.1   | Workspace skeleton + CI matrix | 1 | 3 |
| 1     | Core abstractions (Fetcher, Streamer, traits, runtime) | 2–5 | 8 |
| 2     | Storage engines (CH + PG specialist split) | 6–10 | 13 |
| 3     | First providers | 11–13 | 16 |
| 4     | Hybrid retrieval (Qdrant + Meili + 3 embedders) | 14–20 | 23 |
| 5     | Four consumer shells | 21–26 | 29 |
| 6     | Hardening & docs | 27–32 | 35 |
| 7     | Data engineering (dbt, SQL, ETL/ELT, DDL codegen) | 33–42 | 45 |
| 8     | Agent schemas (12 types, MCP tools, eval runner) | 43–49 | 52 |
| **9** | **Snapshots / time travel / tags / notifications** | **50–58** | **61** |
| **10** | **Streams / CDC / live queries** | **59–66** | **69** |
| **11** | **Graph + spatial** | **67–74** | **77** |
| **12** | **Stages + COPY INTO + open table formats + pipes** | **75–84** | **87** |
| **13** | **UDFs + auth + DEFINE schema + masking** | **85–95** | **97** |

Total ~97 days serial; ~70 days with smart parallelization (Phase 9 can overlap with Phase 7; Phases 11+12 can run alongside 13).

**Recommendation**: ship Phase 9 + 10 first (snapshots + streams + live queries are the highest-leverage features). Phases 11/12/13 can defer to v0.2 if priorities shift.

---

## 12. Open Questions (in addition to ADR follow-ups)

- **O11** — Iceberg write support at v0.1 or v0.2?
- **O12** — Apache AGE vs native Rust graph engine? (deferred to ADR-0015)
- **O13** — SurrealML in `tdw-eval-runner` (Phase 8) or its own `tdw-ml-registry` crate?
- **O14** — Auto MV rewrite optional or P1?
- **O15** — Multi-model unified query (S-8) — design or defer?
- **O16** — Streams transport: PG logical replication + CH versionedCollapsingMergeTree, or homegrown WAL reader?
- **O17** — Should the DEFINE-style schema DSL be YAML, Pkl, or a custom Rust-derived DSL? (ADR-0019)

---

## 13. Changelog

**2026-05-21 — initial direct-mode plan (Layer C feature parity)**
- 438-feature mapping table across Databend (210) + SurrealDB (228), each row classified ✓/◐/✗/—.
- 14 new crates (tdw-snapshot, tdw-stream, tdw-live, tdw-graph, tdw-spatial, tdw-stage, tdw-table-format, tdw-udf + Python/JS/WASM/External variants, tdw-auth, tdw-define).
- 5 phases (9–13) with detailed implementation steps.
- 38 acceptance criteria (A9.1–A13.18).
- 14 risks (R26–R39) with mitigations.
- 15 verification steps (V33–V47).
- ADR + 7 open questions + 5 follow-up ADRs (0015–0019).
- Updated combined timeline: parent (52 days) + this extension (~45 days) = **~97 days serial / ~70 parallelized**.
