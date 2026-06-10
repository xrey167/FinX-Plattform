<!-- Parent: ../AGENTS.md -->
<!-- Generated: 2026-05-27 | Updated: 2026-05-27 -->

# crates

## Purpose

Members of the `FinX-Plattform` Cargo workspace. Every crate is prefixed
`tdw-*` ("Trading Data Warehouse") — the prefix is the clean-room marker that
separates this codebase from FinX-XR (`finx-*`). The G001-G016 production
push is complete, so new crate work should extend an owned runtime,
transport, persistence, service, or tooling boundary rather than adding
placeholder crates.

The crate set is documented by tranche assignments in
`../docs/quality/crate-readiness/matrix.md`. Historical tranche labels remain
useful for ownership, but current cleanup work should use the actual crate
responsibility and downstream callers as the boundary.

## Crate Catalog

Grouped by role, not by tranche. Each crate is documented in detail in its
`../docs/quality/crate-readiness/<name>.md` worksheet.

### Core contracts (G002 — depended on by everything)

| Crate | Role |
|-------|------|
| `tdw-core/` | `Fetcher` / `Streamer` traits, `OBBject<T>` envelope, `Credentials`, `ProviderRegistry`, the five storage-engine traits (`OlapEngine`, `RelationalEngine`, `VectorEngine`, `LexicalEngine`, `BlobEngine`), `WriteSink<T>`, `ProgressOrResult<T>`. |
| `tdw-domain/` | Canonical Rust structs derived from the 11 BOM schemas (`MarketDataBar`, `OrderEvent`, `PositionSnapshot`, `NewsSentiment`, …). |
| `tdw-protocol/` | I/O-free operation, event, approval, queue, and replay contracts shared by service / worker / CLI / MCP. |
| `tdw-config/` | Layered TDW configuration and schema output. |
| `tdw-event/` | `EventEnvelope<P>`, `Actor` enum, `Origin`, `TraceContext` — the wire shape of the event spine (Layer E). |
| `tdw-actor/` | `task_local!` plumbing for the `Actor` context; capability tokens. |
| `tdw-session/` | SQLx/SQLite hot session, permission, approval, and cost state. |
| `tdw-snapshot/` | Snapshot / time-travel scaffolding. |

### Runtime, services, and shells (G007)

| Crate | Role |
|-------|------|
| `tdw-runtime/` | `CommandRunner::run` + `run_streaming` — the orchestrator shared by every consumer shell. |
| `tdw-exec/` | Headless protocol-event execution path. |
| `tdw-app-server/` | Daemon endpoint and submission queue. |
| `tdw-app-client/` | Thin-client contracts that mirror `tdw-app-server`. |
| `tdw-service/` | axum HTTP + tonic gRPC service binary. |
| `tdw-service-api/` | Service-side glue type for the `tdw-app-*` pair. |
| `tdw-mcp/` | MCP server binary (stdio + HTTP/SSE) — exposes Fetchers and Streamers as tools, re-emits streaming progress as MCP notifications. |
| `tdw-acp/` | Outward Agent Client Protocol boundary for future IDE / TUI clients. |
| `tdw-cli/` | Local CLI binary that drives `CommandRunner` directly. |
| `tdw-tui/` | ratatui `EventMsg` renderer. |
| `tdw-worker/` | RiverQueue-backed worker binary; same `CommandRunner` code path as the service. |
| `tdw-rollout/` | Append-only JSONL replay archive. |
| `tdw-replay/` | Event-archive replay tooling (CLI + cursor management). |

### Event spine (G013, Layer E)

| Crate | Role |
|-------|------|
| `tdw-bus/` | In-process `tokio::sync::broadcast` per event-kind. |
| `tdw-outbox/` | Postgres outbox table + RiverQueue publisher worker; atomicity guarantees. |
| `tdw-cdc/` | Logical-replication consumer (`pg_replicate`) + `PgListener` fallback + CH `MaterializedView` bridge. |
| `tdw-hooks/` | `#[hook]` proc-macro, `linkme` registry, sync/async hook dispatch with priority + tx-mode + side flags. |
| `tdw-define/` | DEFINE EVENT declarative wrapper that compiles to `tdw-hooks` registrations. |

### Storage engines (G003 / G010)

| Crate | Role |
|-------|------|
| `tdw-storage-clickhouse/` | `OlapEngine` + `WriteSink` for time-series (OHLCV, ticks, panels). |
| `tdw-storage-postgres/` | `RelationalEngine` + `WriteSink` for OLTP-shaped reference and metadata. |
| `tdw-storage-qdrant/` | `VectorEngine` for embeddings. |
| `tdw-storage-meilisearch/` | `LexicalEngine` for full-text search. |
| `tdw-storage-s3/` | `BlobEngine` for object storage (research notes, eval artifacts). |
| `tdw-storage-parquet/` | `OlapEngine` for cold Parquet files. |
| `tdw-storage-fs/` | Local-filesystem storage adapter — reference impl + offline tests. |
| `tdw-storage-router/` | Type → engine routing rules and `WriteSink` fan-out config. |
| `tdw-stage/` | Stage / open-table-format glue. |
| `tdw-table-format/` | Table-format primitives consumed by `tdw-stage` and storage adapters. |

### Data engineering (G003)

| Crate | Role |
|-------|------|
| `tdw-pipe/` | Pipe / pipeline primitives. |
| `tdw-pipeline/` | Declarative ingest DAG runner (TOML schedules). |
| `tdw-dbt-runner/` | Invokes dbt CLI via `std::process`; parses `target/run_results.json`. |
| `tdw-sql-codegen/` | Derives idempotent Postgres + ClickHouse DDL from `tdw-domain` structs. |
| `tdw-migration/` | sqlx (Postgres) + Refinery (ClickHouse) migration wrapper. |
| `tdw-rewrite/` | SQL rewriting / planner-side transforms. |

### Providers — Fetcher / Streamer impls (G004)

| Crate | Role |
|-------|------|
| `tdw-provider-fileset/` | CSV / Parquet local-file Fetcher (offline reference). |
| `tdw-provider-yahoo/` | Yahoo Finance Fetcher. |
| `tdw-provider-polygon/` | Polygon HTTP Fetcher. |
| `tdw-provider-alpaca/` | Alpaca HTTP Fetcher. |
| `tdw-provider-binance/` | Binance Fetcher. |
| `tdw-provider-fred/` | FRED macro-data Fetcher. |
| `tdw-provider-huggingface/` | HuggingFace dataset Fetcher. |
| `tdw-provider-ws-mock/` | `Streamer` trait skeleton against a fixture WS server. |

**Permanent non-goal:** no `tdw-provider-openbb`. OpenBB is inspiration only.
See ADR-0001 and the clean-room rule.

### LLM + embedding adapters (G004)

| Crate | Role |
|-------|------|
| `tdw-llm/` | `LlmAdapter` trait + deterministic adapter contracts. |
| `tdw-llm-anthropic/` | Anthropic Claude adapter. |
| `tdw-llm-openai-compat/` | OpenAI-compatible adapter (also serves local/llama.cpp endpoints). |
| `tdw-embed/` | `EmbeddingProvider` trait + dispatcher. |
| `tdw-embed-local/` | fastembed-rs / candle (bge-large-en-v1.5 default). |
| `tdw-embed-openai/` | OpenAI `text-embedding-3-large`. |
| `tdw-embed-google/` | Google `text-embedding-004`. |

### Agent infrastructure (G005 / G006, Layer B)

| Crate | Role |
|-------|------|
| `tdw-agent/` | The 9-schema agent type system (`AgentCard`, `AgentSkill`, `SlashCommand`, `Gotcha`, `WorkflowDefinition`, …) with `JsonSchema` + `Validate` derives. |
| `tdw-agent-store/` | Persistence adapters for `tdw-agent` (Postgres + Qdrant + Meilisearch). |
| `tdw-eval-runner/` | Runs `EvalRunRequest` against an `Agent`; emits `EvalRun` rows. |
| `tdw-workflow-engine/` | Validates and executes `WorkflowDefinition` DAGs as RiverQueue jobs. |
| `tdw-tools/` | Tool registry + router + orchestrator contracts. |
| `tdw-auth/` | Authentication primitives. |
| `tdw-auth-oidc/` | OIDC adapter for `tdw-auth`. |
| `tdw-mask/` | Field-masking / RLS as a `Filter`-kind sync hook. |
| `tdw-sandbox/` | UDF runtime sandboxing (cap-std). |
| `tdw-udf/` + `tdw-udf-{js,python,wasm,external}/` | UDF runtimes — JS, Python, WASM, external-process. |

### Knowledge, KG, tags, ML (G006)

| Crate | Role |
|-------|------|
| `tdw-knowledge/` | Retrieval facade over embeddings + vector storage + KG + tags + syntax summaries. |
| `tdw-retrieve/` | Hybrid retriever: vector + lexical + tag channels fused with RRF, as_of filtering, explained graph expansion (B4). |
| `tdw-taxonomy/` | Unified entity taxonomy: 50-kind registry, facets, Origin (tier×source); leaf crate shared by agent + warehouse planes. |
| `tdw-kg/` | Knowledge-graph primitives. |
| `tdw-tags/` | Tag taxonomy. |
| `tdw-tag-rules/` | Tag-rule engine. |
| `tdw-entity-resolver/` | Entity resolution over `tdw-kg`. |
| `tdw-feature-store/` | Feature-store façade for ML signals. |
| `tdw-ml-registry/` | Model registry. |
| `tdw-graph/` | Generic graph primitives. |
| `tdw-storage-graph/` | GraphEngine backends: in-memory reference + cross-backend conformance suite (Bolt backend lands in A4). |
| `tdw-spatial/` | Spatial / geo primitives. |
| `tdw-fn-string/` | Reusable string functions (UDF-shaped). |

### Test + dev utilities

| Crate | Role |
|-------|------|
| `tdw-test-utils/` | Deterministic fixtures and container helpers. |

## Working In This Directory

- **One crate per task.** Crates are the unit of ownership in tranche audits.
  A change that touches three crates probably needs three commits or a single
  PR with explicit scope `(tdw-a, tdw-b, tdw-c)`.
- **Do not add placeholder crates.** If a new crate is needed, give it a real
  contract, tests, docs/worksheet coverage, and a caller. If an old crate looks
  empty or duplicated, verify `cargo metadata`, dependency edges, and its
  readiness worksheet before deleting or merging it.
- **Add new crates by**:
  1. Creating `crates/<name>/Cargo.toml` + `src/lib.rs` (or `src/main.rs` for
     binaries).
  2. Confirming it is picked up by root `Cargo.toml` workspace membership
     (`crates/*`) and adding a `[workspace.dependencies]` entry only when
     other crates should depend on it by workspace alias.
  3. Adding a row to `../docs/quality/crate-readiness/matrix.md` and a
     worksheet `../docs/quality/crate-readiness/<name>.md`.
- **Dependency direction is enforced.** `tdw-core` and `tdw-domain` depend on
  nothing inside the workspace. Storage adapters and providers depend on
  `tdw-core` + `tdw-domain`. Shells (`tdw-service`, `tdw-worker`, `tdw-mcp`,
  `tdw-cli`) depend on `tdw-runtime`. Circular deps are rejected by
  `cargo check --workspace`.

## Testing

Per-crate tests live under `crates/<name>/tests/` (integration / golden) and
inline `#[cfg(test)]` modules in `src/lib.rs`. Run from the workspace root:

```powershell
cargo test --workspace
cargo test -p tdw-core
cargo test -p tdw-domain --test golden_bom
```

## Dependencies

### Internal

- Crates depend on each other via the explicit
  `[workspace.dependencies]` table in `../Cargo.toml`. Do not use bare
  `path = ".."` in a crate manifest — go through the workspace table.

### External (workspace-pinned)

- `async-trait`, `bytes`, `futures-core`, `tokio` — async core.
- `serde`, `serde_json`, `schemars`, `validator`, `thiserror`, `toml` — data
  shape & error handling.
- `reqwest` (rustls), `sqlx` (sqlite-bundled by default) — I/O.
- `ratatui` — TUI.
- `ulid`, `uuid` (v7, serde) — IDs.

<!-- MANUAL: Notes added below this line are preserved on regeneration. -->
