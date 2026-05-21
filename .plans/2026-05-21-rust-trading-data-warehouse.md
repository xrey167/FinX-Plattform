# FinX-Finance — Rust Trading Data Warehouse — Implementation Plan

**Project location:** `C:\Users\ReyDa\FinX-Finance\`
**Crate prefix:** `tdw-*` (Trading Data Warehouse — distinct from FinX-XR's `finx-*`)
**Date:** 2026-05-21
**Mode:** `/omc-plan --direct` → `--consensus` (Architect + Critic reviewed)
**Status:** v2 — consensus improvements applied; ready for Phase 0 execution

---

## 0. Relationship to FinX-XR

**This is a clean-room redesign.** FinX-XR (`C:\Users\ReyDa\Documents\Claude\Projects\FinX\FinX-XR\`)

**Decision (user, 2026-05-21):** keep this as a separate project — a **clean-room redesign** of the same problem space, unconstrained by FinX-XR's accumulated decisions.

**Boundary rules:**
- **No code-sharing in either direction.** Not even copy-paste of trait signatures. Re-derive everything from the OpenBB pattern, the user's 11 BOM schemas, and first principles.
- **Architecture pattern inspiration is permitted.** Reading FinX-XR's roadmap and module layout to understand what worked and what didn't is normal due diligence. Reading code with intent to translate is not.
- **License targets differ.** **FinX-Finance license target = personal** (user decision 2026-05-21: not OSS, not commercial; private codebase for the user's own use). FinX-XR is KV Capital proprietary. This separation is deliberate; do not blur it.
- **Naming separation is enforced by the `tdw-*` crate prefix.** No `finx-*` crate names in this workspace. Imports from `finx-*` are forbidden in `Cargo.toml`.
- **If clean-room v2 ever wants to backport into FinX-XR, that is a future port, not a present-day dependency.**

---

## 1. RALPLAN-DR Summary

### Principles
1. **Specialists with boring wire protocols** beat multi-model generalists (Postgres, ClickHouse, Qdrant, Meilisearch — not Surreal/TypeDB).
2. **One runtime, many shells**: service, worker, MCP, CLI all drive the same orchestrator; never duplicate fetch logic per consumer.
3. **OpenBB pattern + persistence extension**, not a fork: same `Fetcher<Q,D>` contract, but `StorageEngine` is a peer abstraction OpenBB lacks.
4. **Compile-time-or-explicit registration honesty**: explicit `register_provider()` is the primary path; `inventory::submit!` is an optional convenience that may not work under Windows-MSVC + `lto=fat` + `panic=abort` (see R6).
5. **Domain shape is fixed by the 11 BOM schemas**, not by OpenBB's taxonomy. OpenBB is at most one bridge provider, never the surface area.
6. **Clean-room w.r.t. FinX-XR** — see §0.

### Decision Drivers (top 3)
1. Trading data spans **heterogeneous shapes** (timeseries, OLTP reference, blobs, embeddings, lexical) — a clean storage-engine abstraction is non-negotiable.
2. Layer must serve **four personas** (service, worker, agent, library) without divergent codepaths.
3. Layer must be **operable on the user's Windows 11 dev machine** with Docker Desktop / WSL2 — not just on Linux CI.

### Viable Options

#### Option A — Port OpenBB pattern to Rust + add `StorageEngine` peer abstraction *(chosen)*
- **Pros**: proven design pattern, type-safe Rust port, downstream-compatible with future barter-rs / tesser-style consumers, clean `Streamer` sibling for live feeds.
- **Cons**: more crates (~16-20), upfront discipline to keep trait shapes honest.

#### Option B — Pure event-sourced command bus
- **Pros**: clean audit trail, natural replay, decouples producers from consumers.
- **Cons**: latency penalty for synchronous reads, more infra (broker), harder for "fetch and read back" workflows that LLM agents dominate, redundant with what ClickHouse + replay tables already give us.
- **Invalidation rationale**: B over-engineers for a non-existent strict-replay requirement and hurts the LLM-agent loop where synchronous fetch→store→read is dominant.

---

## 2. Requirements Summary

Build a **Rust-native data layer** at `C:\Users\ReyDa\FinX-Finance\` that:

1. **Mirrors the OpenBB Platform provider/fetcher pattern** — generic `Fetcher<Q, D>` contract, **plus** a sibling `Streamer<Q, D>` for live/WS providers, standardized domain models, OBBject-style envelope, pluggable provider registry.
2. **Persists** what OpenBB does not: timeseries (OHLCV, ticks, intraday), alternative data (sentiment, macro, alt panels), research artifacts (studies, speeches, filings, transcripts, PDFs), reference data, and embeddings.
3. **Uses ClickHouse + PostgreSQL at the baseline**, with **per-engine specialist traits** (`OlapEngine`, `RelationalEngine`, `VectorEngine`, `LexicalEngine`, `BlobEngine`) and a shared `WriteSink<T>` for routing fan-out.
4. **Exposes the layer four ways**: native Rust library, HTTP/gRPC service, background worker, and MCP/agent tool surface — with one canonical streaming-aware `CommandRunner` underneath all four.
5. **Integrates the user's existing 11-schema BOM** (market data, orders, positions, news/sentiment, fundamentals, strategy, risk, time/calendar, ops, ref data, costs) as the canonical domain model — **subject to BOM-schema location confirmation in Phase 0.0** (see open question O1).
6. **Renders explicit verdicts** on the 14 candidate Rust DB/search repos.
7. **Targets Windows 11 + Docker Desktop / WSL2** as the primary dev environment, with Linux CI as secondary.

### Non-goals

- Not building a trading engine, OMS, backtester, or strategy framework.
- Not a frontend (DBeaver / DataGrip / open-db-studio cover ad-hoc UI).
- Not replacing FinX-XR; not depending on it; not importing from it.
- **No `tdw-provider-openbb` bridge — ever.** User decision 2026-05-21: OpenBB is *permanently* inspiration only. Build a better domain model from the 11 BOM schemas; do not bridge to a running OpenBB instance. This is a permanent non-goal, not a v2-deferred item.

---

## 3. Acceptance Criteria

A1. `cargo build --workspace --release` succeeds on **Windows-MSVC** with the release profile (`lto = "fat"`, `panic = "abort"`, `codegen-units = 1`); `cargo test --workspace` passes.
A2. `Fetcher<Q, D>` **and** `Streamer<Q, D>` traits exist in `tdw-core` — the former for request-response, the latter for live feeds. Both ship at Phase 1 even if `Streamer` has zero impls.
A3. At least **two `Fetcher` providers** implement the same domain endpoint (`equity_historical`) — one HTTP (e.g. Polygon or yfinance proxy), one fileset (CSV/Parquet). At least **one `Streamer` skeleton provider** demonstrates the trait shape (mock WS, no third-party dependency).
A4. **Specialist storage traits** — `OlapEngine`, `RelationalEngine`, `VectorEngine`, `LexicalEngine`, `BlobEngine` — each implemented by exactly one engine. A shared `WriteSink<T>` trait routes writes across them; per-engine read traits remain specialist.
A5. **One axum HTTP endpoint** (`GET /v1/equity/historical?provider=...`) returns standardized JSON with provider selection via query param.
A6. **One worker** runs a scheduled pull using **RiverQueue** (Postgres-backed) — same `CommandRunner` code path as the service.
A7. **One MCP tool** exposes the equity historical fetcher to LLM agents with JSON-Schema-generated input, **and supports streaming progress notifications** via `CommandRunner::run_streaming` (see A9).
A8. **Qdrant + Meilisearch** integrated for `research_note`: ingest pipeline embeds + indexes; hybrid search endpoint returns top-k with score-fusion explanations. **Embedding provider is pluggable** via an `EmbeddingProvider` trait, with three reference impls shipping at Phase 4: `tdw-embed-local` (fastembed-rs / candle, e.g. bge-large-en-v1.5), `tdw-embed-openai` (text-embedding-3-large), `tdw-embed-google` (text-embedding-004 / gemini-embedding-001). Choice is config-driven at setup; Qdrant collections are named per-model so multiple can coexist or migrate.
A9. `CommandRunner::run_streaming(...)` returns `impl Stream<Item = ProgressOrResult<D>>` from day one. The HTTP service consumes the terminal `Result`; MCP consumes the full stream and re-emits as `notifications/progress`.
A10. **Provider registration honesty**: the **primary** registration path is an explicit `register_provider(&mut registry, ProviderFactory)` called from `main`. `inventory::submit!` is an **optional** convenience available behind feature flag `inventory-registration`. **A9 is verified by a CI job that runs provider discovery on Windows-MSVC + release profile (`lto = "fat"`, `panic = "abort"`), not just Linux debug.**
A11. **Workload-anchored SLOs** (replacing the original arbitrary p95 numbers): bench harness in `xtask/bench.rs` records throughput and p50/p95/p99 for three named workloads — `historical_aapl_5y_daily_clickhouse_warm`, `ingest_1e6_rows_clickhouse_single_node`, `qdrant_knn_1m_points_top10` — against a pinned dev machine spec. Numbers are reported and tracked in `docs/perf-history.md`; CI fails on >20% regression vs the rolling 7-day baseline. **There are no absolute targets in v0.1**; targets are set after the first three weeks of measurement.
A12. **Repo verdict matrix** committed as `docs/repo-verdicts.md` with rationale + integration sketch for each of the 14 candidates.
A13. **Docker Compose with profiles**: `--profile minimal` brings up Postgres + ClickHouse (covers ~70% of dev). `--profile full` adds Qdrant + Meilisearch + MinIO + Redis. Documented WSL2-hosted-volumes path for Windows users to avoid bind-mount slowness.

---

## 4. Architecture Overview

```
                                ┌─────────────────────────────────────────────┐
                                │  CONSUMERS                                  │
                                │  ─ axum HTTP / tonic gRPC service           │
                                │  ─ Apalis worker (scheduled + on-demand)    │
                                │  ─ MCP server (streaming-aware tool surface)│
                                │  ─ Rust lib (direct embedding by downstream)│
                                └────────────────────┬────────────────────────┘
                                                     │
                                                     ▼
              ┌──────────────────────────────────────────────────────────────────┐
              │  COMMAND RUNNER  (tdw-runtime)                                   │
              │  ProviderRegistry → resolve(domain, endpoint, provider)          │
              │  → Fetcher::fetch() OR Streamer::stream()                        │
              │  → emits Stream<ProgressOrResult<D>>                             │
              │  → Optional StoragePipeline::write(OBBject) via WriteSink router │
              └────────┬──────────────────────────────────────┬──────────────────┘
                       │                                      │
                       ▼                                      ▼
        ┌──────────────────────────┐         ┌────────────────────────────────────┐
        │  PROVIDERS               │         │  STORAGE ENGINES                   │
        │  (Fetcher<Q,D>           │         │  (specialist traits + WriteSink<T>)│
        │   or Streamer<Q,D>)      │         │                                    │
        │                          │         │  tdw-storage-clickhouse  : OlapEng │
        │  tdw-provider-fileset    │         │  tdw-storage-postgres    : RelEng  │
        │  tdw-provider-polygon    │         │  tdw-storage-qdrant      : VecEng  │
        │  tdw-provider-yahoo      │         │  tdw-storage-meilisearch : LexEng  │
        │  tdw-provider-fred       │         │  tdw-storage-s3          : BlobEng │
        │  tdw-provider-alpaca     │         │  tdw-storage-parquet     : OlapEng │
        │  tdw-provider-binance    │         │                            (cold)  │
        │  tdw-provider-ws-mock    │         │  [v2: tdw-storage-risingwave]      │
        │  …                       │         │                                    │
        └──────────────────────────┘         └────────────────────────────────────┘
                       │                                      │
                       ▼                                      ▼
              ┌───────────────────────────────────────────────────────────┐
              │  DOMAIN MODELS  (tdw-domain)                              │
              │  EquityHistoricalData, BalanceSheetData, EconomicCal…,    │
              │  NewsItem, ResearchNote, SpeechTranscript, RiskMetric…    │
              │  — sourced from the 11 BOM schemas (Phase 0.0 confirms    │
              │    location; Phase 1.4 imports as Rust structs)           │
              └───────────────────────────────────────────────────────────┘
                       │                                      │
                       ▼                                      ▼
                          ┌───────────────────────────────┐
                          │  CORE  (tdw-core)             │
                          │  Fetcher<Q,D>, Streamer<Q,D>, │
                          │  OBBject<T>, QueryParams,     │
                          │  Data, WriteSink<T>,          │
                          │  CredentialStore, Errors      │
                          └───────────────────────────────┘
```

### Key contracts (Rust mapping of OpenBB primitives, with consensus refinements)

```rust
// tdw-core/src/fetcher.rs
#[async_trait]
pub trait Fetcher: Send + Sync + 'static {
    type Query: QueryParams;
    type Data:  DataModel;
    const PROVIDER: &'static str;
    const ENDPOINT: &'static str;

    fn transform_query(params: serde_json::Value) -> Result<Self::Query>;
    async fn extract_data(query: &Self::Query, creds: &Credentials) -> Result<Bytes>;
    fn transform_data(query: &Self::Query, raw: Bytes) -> Result<Vec<Self::Data>>;

    async fn fetch(params: serde_json::Value, creds: &Credentials) -> Result<OBBject<Self::Data>> {
        let q = Self::transform_query(params)?;
        let raw = Self::extract_data(&q, creds).await?;
        let rows = Self::transform_data(&q, raw)?;
        Ok(OBBject::new(rows, Self::PROVIDER, Self::ENDPOINT))
    }
}

// tdw-core/src/streamer.rs   (NEW vs v1 — Tension 1 resolution)
#[async_trait]
pub trait Streamer: Send + Sync + 'static {
    type Query: QueryParams;
    type Data:  DataModel;
    const PROVIDER: &'static str;
    const ENDPOINT: &'static str;

    async fn subscribe(
        query: Self::Query,
        creds: &Credentials,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<Self::Data>> + Send>>>;

    /// Snapshot at subscription time (for state restoration).
    async fn snapshot(query: &Self::Query, creds: &Credentials) -> Result<Vec<Self::Data>>;

    /// Acknowledge processed sequence for resumable streams.
    async fn checkpoint(seq: u64) -> Result<()> { Ok(()) }
}

// tdw-core/src/storage.rs   (Split per Tension 2)
#[async_trait]
pub trait WriteSink<T: DataModel>: Send + Sync {
    fn name(&self) -> &'static str;
    async fn write_batch(&self, batch: &OBBject<T>) -> Result<WriteReceipt>;
    async fn health_check(&self) -> Result<HealthStatus>;
}

#[async_trait]
pub trait OlapEngine: Send + Sync {
    async fn query_sql<T: DataModel>(&self, sql: &str, params: Params) -> Result<OBBject<T>>;
    async fn execute(&self, ddl: &str) -> Result<()>;
}

#[async_trait]
pub trait RelationalEngine: Send + Sync {
    async fn fetch_one<T: DataModel>(&self, sql: &str, params: Params) -> Result<Option<T>>;
    async fn fetch_all<T: DataModel>(&self, sql: &str, params: Params) -> Result<Vec<T>>;
    async fn transaction<F, R>(&self, f: F) -> Result<R> where F: AsyncFnOnce(&Tx) -> Result<R>;
}

#[async_trait]
pub trait VectorEngine: Send + Sync {
    async fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<()>;
    async fn search_knn(&self, collection: &str, query: VectorQuery) -> Result<Vec<ScoredPoint>>;
}

#[async_trait]
pub trait LexicalEngine: Send + Sync {
    async fn index(&self, idx: &str, docs: Vec<LexicalDoc>) -> Result<()>;
    async fn search_text(&self, idx: &str, query: TextQuery) -> Result<Vec<ScoredDoc>>;
}

#[async_trait]
pub trait BlobEngine: Send + Sync {
    async fn put_object(&self, key: &str, body: Bytes, content_type: &str) -> Result<()>;
    async fn get_object(&self, key: &str) -> Result<Bytes>;
    async fn presigned_url(&self, key: &str, ttl: Duration) -> Result<Url>;
}

// tdw-core/src/runtime.rs   (Streaming-aware per Tension 3)
pub enum ProgressOrResult<T> {
    Progress { stage: &'static str, fraction: f32, message: Option<String> },
    Partial(T),
    Done(OBBject<T>),
    Error(Error),
}

impl CommandRunner {
    pub async fn run<F: Fetcher>(&self, params: Value) -> Result<OBBject<F::Data>>;

    pub fn run_streaming<F: Fetcher>(
        &self,
        params: Value,
    ) -> impl Stream<Item = ProgressOrResult<F::Data>>;
}
```

### How OpenBB concepts map (one-to-one, with refinements)

| OpenBB (Python)                              | Rust equivalent (tdw-*) — v2                                                  |
| -------------------------------------------- | ----------------------------------------------------------------------------- |
| `Fetcher[Q, D]` (Generic, ABC)               | `trait Fetcher` (req-resp) + **`trait Streamer` (live feeds)** — sibling, not subtype |
| `QueryParams` (Pydantic BaseModel)           | `trait QueryParams: Serialize + Deserialize + JsonSchema`                     |
| `Data` (Pydantic BaseModel)                  | `trait DataModel: Serialize + Deserialize + Validate + JsonSchema`            |
| `OBBject[T]` (Pydantic generic envelope)     | `struct OBBject<T: DataModel>`                                                |
| `Provider` (dataclass registry container)    | `struct Provider { fetchers: HashMap<&'static str, FetcherFactory> }`         |
| `ProviderRegistry` (entry-point discovery)   | **Explicit `register_provider()` primary; `inventory::submit!` optional**     |
| `CommandRunner.run()` (orchestrator)         | `tdw-runtime::CommandRunner::run()` + `run_streaming()`                       |
| `FastAPI Router`                             | `axum::Router` with one handler per domain                                    |
| `standard_models/`                           | `tdw-domain` crate, one module per BOM schema                                 |
| `pyproject.toml` entry points                | Explicit registration in `main`; `inventory` optional                          |
| *(no equivalent — added)*                    | `WriteSink<T>` + specialist read engines (`OlapEngine` etc.)                   |

---

## 5. Workspace Layout

```
FinX-Finance/                                  # = C:\Users\ReyDa\FinX-Finance\
├── Cargo.toml                                # workspace
├── docker-compose.yaml                       # with --profile minimal / full
├── .plans/                                   # this file lives here
├── docs/
│   ├── architecture.md
│   ├── repo-verdicts.md
│   ├── perf-history.md                       # A11 workload SLO baseline
│   ├── adr/
│   │   ├── 0001-relationship-to-finx-xr.md
│   │   ├── 0002-fetcher-vs-streamer-traits.md
│   │   ├── 0003-storage-trait-split.md
│   │   ├── 0004-mcp-streaming-design.md
│   │   ├── 0005-explicit-vs-inventory-registration.md
│   │   ├── 0006-workload-anchored-slos.md
│   │   ├── 0007-bom-schema-re-derivation.md  # Phase 0.0.2 provenance + drift gate
│   │   ├── 0009-license-personal.md
│   │   └── 0010-no-openbb-bridge.md          # permanent non-goal
│   └── schemas/                              # 11 BOM schemas (or pointer)
│
├── crates/
│   ├── tdw-core/                             # traits (Fetcher, Streamer, all engine traits, WriteSink, OBBject)
│   ├── tdw-domain/                           # 11 BOM schema structs
│   ├── tdw-runtime/                          # CommandRunner (sync + streaming), ProviderRegistry
│   ├── tdw-storage-clickhouse/               # impl OlapEngine + WriteSink<OHLCV-shape>
│   ├── tdw-storage-postgres/                 # impl RelationalEngine + WriteSink<Ref-shape>
│   ├── tdw-storage-qdrant/                   # impl VectorEngine
│   ├── tdw-storage-meilisearch/              # impl LexicalEngine
│   ├── tdw-storage-s3/                       # impl BlobEngine
│   ├── tdw-storage-parquet/                  # impl OlapEngine (cold)
│   ├── tdw-storage-router/                   # type → engine routing rules
│   │
│   ├── tdw-embed/                            # EmbeddingProvider trait + dispatcher
│   ├── tdw-embed-local/                      # fastembed-rs / candle (bge-large-en-v1.5 default)
│   ├── tdw-embed-openai/                     # text-embedding-3-large
│   ├── tdw-embed-google/                     # text-embedding-004 / gemini-embedding-001
│   │
│   ├── tdw-provider-fileset/                 # CSV/Parquet local files (Fetcher)
│   ├── tdw-provider-polygon/
│   ├── tdw-provider-yahoo/
│   ├── tdw-provider-fred/
│   ├── tdw-provider-alpaca/
│   ├── tdw-provider-binance/
│   ├── tdw-provider-ws-mock/                 # Streamer trait demo (NEW)
│   #  (no tdw-provider-openbb — see §3 non-goals)
│   │
│   ├── tdw-pipeline/                         # ingest DAGs, declarative schedules
│   ├── tdw-worker/                           # Apalis worker binary
│   ├── tdw-service/                          # axum + tonic binary
│   ├── tdw-mcp/                              # MCP server (streaming-aware)
│   ├── tdw-cli/                              # local CLI
│   └── tdw-test-utils/                       # fixtures, wiremock tapes, mock providers
│
├── examples/
│   ├── 01_basic_fetch.rs
│   ├── 02_storage_roundtrip.rs
│   ├── 03_worker_pipeline.rs
│   ├── 04_agent_tool_use.rs
│   ├── 05_streaming_provider.rs              # NEW — exercises Streamer trait
│   └── 06_hybrid_search.rs
│
└── xtask/
    └── bench.rs                              # A11 workload-anchored SLO harness
```

**Note**: `tdw-provider-openbb` is **a permanent non-goal** (user decision 2026-05-21). OpenBB is inspiration only; FinX-Finance builds its own domain from the 11 BOM schemas and does not bridge to any OpenBB instance. ADR-0001 records this stance.

---

## 6. Storage Matrix (which DB for what)

Decision rule: **specialists with boring wire protocols beat multi-model generalists.** The trait layer (§4) splits along the same lines (Tension 2 resolution): each engine implements one specialist read trait + the shared `WriteSink<T>`.

| Data class                                | Primary store        | Secondary (mirror) | Specialist trait used | Why                                              |
| ----------------------------------------- | -------------------- | ------------------ | --------------------- | ------------------------------------------------ |
| OHLCV bars, ticks, trades                 | **ClickHouse**       | Parquet (cold)     | `OlapEngine`          | Battle-tested OLAP, columnar compression         |
| Order book snapshots                      | **ClickHouse**       | Parquet (cold)     | `OlapEngine`          | Wide-row compression handles depth levels        |
| Alt-data panels (sentiment, macro, etc.)  | **ClickHouse**       | Parquet (cold)     | `OlapEngine`          | Same access pattern as OHLCV                     |
| Instrument reference, listings            | **Postgres**         | —                  | `RelationalEngine`    | Joins, FKs, transactional updates                |
| Accounts, positions, orders (live OMS)    | **Postgres**         | ClickHouse (audit) | `RelationalEngine`    | OLTP with strict consistency                     |
| Fundamentals, filings metadata            | **Postgres**         | ClickHouse (panels)| `RelationalEngine`    | Hierarchical relational shape                    |
| Calendar / corporate actions              | **Postgres**         | —                  | `RelationalEngine`    | Mostly read, FK-joined heavily                   |
| Research notes (raw PDF blob)             | **S3/MinIO**         | —                  | `BlobEngine`          | Object storage for long-form docs                |
| Research notes (metadata)                 | **Postgres**         | —                  | `RelationalEngine`    | Title, source, date, tickers, FK to S3 key       |
| Research notes (embeddings)               | **Qdrant**           | —                  | `VectorEngine`        | Specialist vector DB, official Rust client       |
| Research notes (lexical full-text)        | **Meilisearch**      | —                  | `LexicalEngine`       | Best-in-class lexical search, MIT, HTTP          |
| News items, headlines                     | CH + Meili + Qdrant  | Postgres metadata  | (multi-trait)         | Time-series + lexical + semantic, all matter     |
| Strategy outputs, signals                 | **ClickHouse**       | —                  | `OlapEngine`          | Time-series-shaped                               |
| Risk metrics (VaR/Greeks panels)          | **ClickHouse**       | —                  | `OlapEngine`          | Time-series, wide                                |
| Live tick streams (post-Phase 4)          | **ClickHouse** (sink)| RisingWave (v2)    | `OlapEngine` + future | Tick fan-in → bar aggregator → CH; RW later      |
| Job queue, worker state                   | **Postgres**         | Redis (cache)      | `RelationalEngine`    | Apalis-postgres; transactional job semantics     |
| Cache, pub-sub, ephemeral                 | **Redis**            | —                  | (separate `KvStore`)  | Standard                                         |

### Streaming layer (deferred to v2, designed for plug-in)

When live PnL / real-time risk arrives as a real requirement, **RisingWave** drops in (Apache-2.0, distributed, Postgres wire) without disturbing `tdw-domain`. Materialize is the second choice (BSL 1.1, picked only if exact correctness semantics needed). See §7 verdict matrix.

---

## 7. The 14-Repo Verdict Matrix

| # | Repo | Verdict | Role | Rationale |
|---|------|---------|------|-----------|
| 1 | `typedb/typedb` | **SKIP** | — | Custom TypeQL = schema lock-in; Postgres FKs cover financial reference. |
| 2 | `MaterializeInc/materialize` | **OPTIONAL** | Streaming (v2 alt) | BSL 1.1. Pick RisingWave instead unless strict correctness semantics matter. |
| 3 | `risingwavelabs/risingwave` | **OPTIONAL** | Streaming (v2 preferred) | Apache-2.0, distributed, Postgres wire. **Preferred streaming engine** when streaming is real. |
| 4 | `databendlabs/databend` | **SKIP** | — | Duplicates ClickHouse. |
| 5 | `neondatabase/neon` | **OPTIONAL** (managed only) | Dev branching | Self-host = too much ops. Managed = transparent Postgres wire. |
| 6 | `qdrant/qdrant` | **USE** ✓ | `tdw-storage-qdrant` | Official Rust client, gRPC, mature, beats pgvector at scale. |
| 7 | `influxdata/influxdb` (v3) | **SKIP** | — | Duplicates ClickHouse with worse SQL ergonomics. |
| 8 | `surrealdb/surrealdb` | **SKIP** | — | Jack-of-all-trades + BSL 1.1. Specialists win. |
| 9 | `meilisearch/meilisearch` | **USE** ✓ | `tdw-storage-meilisearch` | MIT community ed., HTTP API, official Rust SDK. Pairs with Qdrant for hybrid. |
| 10 | `meadowlark-bradsher/Tee` | **SKIP** | — | Pre-alpha personal project, no license. |
| 11 | `wallfacers/open-db-studio` | **TOOL ONLY** | Dev convenience | Tauri+Rust+React DB IDE. Not runtime. |
| 12 | `trailbaseio/trailbase` | **SKIP** | — | Wrong shape (SQLite-BaaS); OSL-3.0 server license copyleft trap. |
| 13 | `GreptimeTeam/greptimedb` | **OPTIONAL** (v2) | Future CH alt | Apache-2.0, Postgres+MySQL wire, object-store native. Track for v2. |
| 14 | `readysettech/readyset` | **OPTIONAL** (later) | Postgres read cache | BSL 1.1. Drop-in. Adopt only when measured PG read latency is a bottleneck. |

**v0.1 adds**: Qdrant, Meilisearch.
**v2 slots**: RisingWave (streaming), Readyset (read cache), GreptimeDB (cold TS).
**Never**: TypeDB, Databend, InfluxDB v3, SurrealDB, Tee, TrailBase.

`docs/repo-verdicts.md` reproduces this with per-row integration sketches.

---

## 8. Service / Pipeline / Worker / Agent Integration

### 8.1 Service (HTTP / gRPC) — `tdw-service`
- **axum** HTTP, **tonic** gRPC. One handler per domain endpoint.
- Routes mirror OpenBB: `GET /v1/equity/historical?symbol=AAPL&provider=polygon&start_date=...`.
- `?provider=` resolves via `ProviderRegistry`; defaults to first-registered.
- `?store=true` triggers `StoragePipeline::write` via `WriteSink` router — same code path as the worker.
- gRPC defs auto-generated from domain models via `prost-build`.
- HTTP layer consumes only the **terminal `Result`** from `CommandRunner::run_streaming` (drops progress events).

### 8.2 Pipeline (declarative DAGs) — `tdw-pipeline`
- TOML schedules:
  ```toml
  [[job]]
  name        = "polygon_eod_equities"
  schedule    = "0 22 * * 1-5"
  fetcher     = "polygon::equity_historical"
  params_for  = "all_listed_us_equities"
  storage     = ["clickhouse:hot", "parquet:cold"]
  retries     = 3
  backoff_ms  = 5000
  ```
- Compiles to Apalis job structs.
- Cross-engine fan-out (research-note → embed/index/blob/metadata) is one declarative pipeline, atomic with rollback.

### 8.3 Worker — `tdw-worker`
- **RiverQueue (Rust port) as the chosen runtime** (user decision 2026-05-21). Postgres-backed, transactional enqueue semantics, explicit unique-job constraints, well-defined schema/protocol.
- Worker runs scheduled, on-demand, and reactive jobs through the same `CommandRunner`.
- ADR-0008 records the choice + the rejected Apalis alternative for future reference.

### 8.4 Agent / MCP — `tdw-mcp` (streaming-aware)
- MCP server (stdio + HTTP/SSE) exposes Fetchers + Streamers as tools.
- Tool schemas auto-generated from `Query` (via `schemars` `JsonSchema` derive).
- **Streaming progress** from `CommandRunner::run_streaming` re-emitted as MCP `notifications/progress`.
- MCP-specific error mapping lives in `tdw-mcp`, not polluting `tdw-core::Error`. ADR-0004 records this boundary.
- Special tools:
  - `tdw.documents.search_hybrid` — Qdrant + Meilisearch fusion with RRF scoring.
  - `tdw.documents.ingest` — accept PDF/URL/text → blob + embed + index + metadata atomic write.
- LLM agents (Claude Code, OpenAI agents, custom) get a first-class tool surface.

### One runtime, four shells
```
                  ┌─────────────────────┐
                  │   tdw-runtime       │
                  │   CommandRunner     │
                  │   (run, run_stream) │
                  └─────┬───────────────┘
                        │ same code
        ┌───────────────┼───────────────┬──────────────┐
        ▼               ▼               ▼              ▼
   tdw-service     tdw-worker      tdw-mcp        tdw-cli
   (HTTP/gRPC)     (Apalis)        (streaming)    (local)
```

---

## 9. Implementation Phases

### Phase 0.0 — Discovery (before any code) — Day 0–2
0.0.1. Read `FinX-XR/docs/tasks/openbb-parity-roadmap.md` — **for context only, no copying**. Note what worked, what didn't.
0.0.2. **Re-derive the 11 BOM schemas from scratch** (user decision 2026-05-21: prior synthesis is lost; do not search; re-synthesize cleanly). Curate a fresh source list of public/OSS trading projects (e.g. barter-rs, OrderBook-rs, ta-rs, RustQuant, ccxt, freqtrade, OpenBB standard_models, FIX 4.4 / FIX 5.0 SP2 dictionaries, ISO 20022 financial-instrument message catalog). Run a parallel-agent synthesis pass producing the 11 schemas as markdown specs in `docs/schemas/01_market_data.md` through `docs/schemas/11_costs_fees.md`, each with 15-25 sections + pitfalls. Document the curated source list in `docs/schemas/00_provenance.md`. **No FinX-XR sources in the corpus.**
0.0.3. Inspect `references/tesser/tesser-data/` for **pattern inspiration only** on the `Streamer<Q,D>` trait shape. Re-derive; do not import. (FinX-XR's `finx-data` is off-limits — clean-room boundary.)
0.0.4. Confirm Windows 11 + Docker Desktop / WSL2 status. Validate that ClickHouse + Postgres containers come up with reasonable startup times under WSL2-hosted volumes.
0.0.5. License = **personal** (user decision 2026-05-21). Record in ADR-0009. Not OSS, not commercial; private codebase for the user.

**Exit criteria**: ADR-0001 signed; 11 BOM schema markdown files written to `docs/schemas/`; `docs/schemas/00_provenance.md` lists the curated OSS source list; ADR-0009 records the personal license; Windows-Docker status report written.

### Phase 0.1 — Workspace skeleton — Day 1
0.1.1. Cargo workspace at `C:\Users\ReyDa\FinX-Finance\` with all crate stubs (compile but empty).
0.1.2. `docker-compose.yaml` with `--profile minimal` (Postgres + ClickHouse) and `--profile full` (adds Qdrant + Meilisearch + MinIO + Redis). WSL2-hosted-volumes documented for Windows.
0.1.3. `tdw-test-utils` with `testcontainers-rs`, gated by profile, slow-test annotations for Windows.
0.1.4. `xtask` with bench harness skeleton.
0.1.5. CI: GitHub Actions matrix — Ubuntu (debug + release) **and Windows-MSVC (release with `lto=fat`, `panic=abort`)**. `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`.

**Exit criteria**: A1, A13 from §3 satisfied; Windows CI green.

### Phase 1 — Core abstractions — Days 2–5
1.1. `tdw-core::fetcher`: `Fetcher<Q,D>` with three methods + default `fetch()`.
1.2. `tdw-core::streamer`: **`Streamer<Q,D>`** (NEW vs v1) with `subscribe`, `snapshot`, `checkpoint`. Even with zero impls in Phase 1, the trait shape is frozen.
1.3. `tdw-core::storage`: `WriteSink<T>`, `OlapEngine`, `RelationalEngine`, `VectorEngine`, `LexicalEngine`, `BlobEngine` — all five specialist traits.
1.4. `tdw-core::runtime`: `ProgressOrResult<T>` enum, `CommandRunner::run_streaming` signature.
1.5. `tdw-core::registry`: explicit `register_provider()` as **primary**; `inventory::submit!` behind `inventory-registration` feature flag.
1.6. `tdw-domain`: import the 11 BOM schemas (location confirmed in 0.0.2) as Rust structs. `#[derive(Serialize, Deserialize, JsonSchema, Validate)]`.
1.7. `tdw-runtime`: `CommandRunner::run` (sync) + `run_streaming` (default impl wraps `run`).
1.8. Unit tests: round-trip `OBBject` JSON; mock `Fetcher` and mock `Streamer`; registry resolves both paths (explicit + inventory feature).

**Exit criteria**: A2, A10 satisfied.

### Phase 2 — Storage engines (specialist split) — Days 6–10
2.1. `tdw-storage-postgres` implements `RelationalEngine` + `WriteSink<T>` for relational shapes.
2.2. `tdw-storage-clickhouse` implements `OlapEngine` + `WriteSink<T>` for timeseries.
2.3. `tdw-storage-router`: TOML routing config, `WriteSink` fan-out per data type.
2.4. Integration tests via `testcontainers`: round-trip `Vec<EquityHistoricalData>` to ClickHouse; `Vec<Instrument>` to Postgres.

**Exit criteria**: A4 satisfied.

### Phase 3 — First providers — Days 11–13
3.1. `tdw-provider-fileset`: CSV/Parquet Fetcher. Offline. Reference impl.
3.2. `tdw-provider-yahoo` or `tdw-provider-polygon`: HTTP Fetcher for `equity_historical`. `reqwest` + `serde`.
3.3. `tdw-provider-ws-mock`: Streamer skeleton against a fixture WS server (using `tungstenite`/`tokio-tungstenite` + an in-process echo server). No third-party WS account needed.
3.4. Both Fetchers register via explicit `register_provider()` (primary) — same provider also registers via `inventory::submit!` if the feature flag is on, exercising both paths.
3.5. Provider tests with `wiremock` golden tapes.

**Exit criteria**: A3 satisfied.

### Phase 4 — Hybrid retrieval + multi-provider embeddings — Days 14–20
4.1. `tdw-storage-qdrant`: `qdrant-client` crate, gRPC, named vectors per embedding model.
4.2. `tdw-storage-meilisearch`: `meilisearch-sdk`.
4.3. `tdw-storage-s3`: aws-sdk-rust, MinIO-compatible.
4.4. **`tdw-embed` trait crate**: define `EmbeddingProvider` trait with `embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>>` + `model_id() -> &str` + `dimension() -> usize`. Dispatcher type that wraps any impl.
4.5. **`tdw-embed-local`**: fastembed-rs default (bge-large-en-v1.5, 1024-dim), candle backend optional for larger models. Offline; no API key needed.
4.6. **`tdw-embed-openai`**: `text-embedding-3-large` via `reqwest`; uses `Credentials::openai_api_key`.
4.7. **`tdw-embed-google`**: `text-embedding-004` (or `gemini-embedding-001`) via `reqwest`; uses `Credentials::google_api_key`.
4.8. **Document ingest pipeline**: PDF/URL/text → extract → embed (provider selected by config at setup) → S3 (blob) + Postgres (metadata) + Qdrant (vectors, collection name = `{schema}__{model_id}`) + Meilisearch (lexical). Atomic with rollback.
4.9. **Hybrid search**: `documents::search_hybrid(query, top_k)` — fanout to Qdrant (using the configured embedding provider to embed the query) + Meilisearch, RRF fusion, returns ranked items with score breakdown.

**Exit criteria**: A8 satisfied.

### Phase 5 — Four consumer shells — Days 19–24
5.1. `tdw-service`: axum HTTP server; one route per domain endpoint; OpenBB-compatible URLs. gRPC mirror via tonic. Consumes terminal `Result` from `run_streaming`.
5.2. `tdw-worker`: Apalis-postgres worker. TOML schedule definitions.
5.3. `tdw-mcp`: MCP server (stdio + HTTP/SSE) exposing Fetchers + Streamers as tools. Streams `notifications/progress`.
5.4. `tdw-cli`: thin CLI driving `CommandRunner` directly.
5.5. **Bench harness** in `xtask/bench.rs` runs three workloads and records to `docs/perf-history.md`. CI regression gate at 20%.

**Exit criteria**: A5, A6, A7, A9, A11 satisfied.

### Phase 6 — Hardening & docs — Days 25–30
6.1. `docs/repo-verdicts.md` finalized (A12).
6.2. `docs/architecture.md` with Mermaid diagrams.
6.3. All ADRs (0001-0007) signed.
6.4. Per-crate `README.md` with usage examples.
6.5. `examples/*.rs` actually run as CI integration tests.
6.6. Credentials handling audited: no plaintext in logs/panic messages; `Credentials` is `Zeroize` on drop; MCP tool surface masks sensitive fields.
6.7. **Provider-registration Windows-MSVC release-profile check** (A10) runs in CI on every PR.

**Exit criteria**: all A1–A13 satisfied.

---

## 10. Risks & Mitigations

| #   | Risk | Likelihood | Impact | Mitigation |
|----|------|-----------|--------|------------|
| R1  | `Fetcher` shape leaks for streaming/WS providers | — | — | **Resolved**: ship `Streamer<Q,D>` sibling trait at Phase 1 (not deferred). |
| R2  | BOM schemas not on disk; user-confirmed prior synthesis is lost | — | — | **Resolved**: Phase 0.0.2 re-synthesizes from a curated OSS-only source list. Markdown specs land in `docs/schemas/`; Rust structs in `tdw-domain` derive from them. CI gate (V14) catches drift between the two. |
| R3  | Docker Compose stack heavy on Windows dev | **High** | Medium | `--profile minimal` defaults; Qdrant/Meili opt-in; documented WSL2-hosted-volumes path. |
| R4  | Embedding model dimension lock | High | Low | **Resolved**: three embedding providers ship at Phase 4 (`tdw-embed-local`, `tdw-embed-openai`, `tdw-embed-google`); Qdrant collections named `{schema}__{model_id}` so multiple coexist; switching = new collection + reindex job, never a schema break. |
| R5  | Apalis vs RiverQueue API ergonomics | — | — | **Resolved**: RiverQueue chosen (user decision 2026-05-21); ADR-0008 records choice + rejected alternative. |
| R6  | **`inventory` crate fails under Windows-MSVC + `lto=fat` + `panic=abort` + `strip=symbols`** | **High** | **High** | Explicit `register_provider()` is the **primary** path; `inventory` behind feature flag. A10 explicitly tests release profile on Windows-MSVC. |
| R7  | OpenBB feature creep ("mirror every endpoint") | Low | Medium | 11 BOM schemas are canonical; **OpenBB bridge is a permanent non-goal** (user decision 2026-05-21); reject endpoints not backed by a BOM schema. |
| R8  | Schema codegen for ClickHouse `CREATE TABLE` becomes a yak-shave | Medium | Medium | Phase 2 hand-written; codegen post-v1. |
| R9  | Accidental code-sharing with FinX-XR (clean-room violation) | Medium | **High** | `Cargo.toml` lints reject any `finx-*` dep; PR review checklist includes "no `finx-*` imports"; ADR-0001 boundary rules; periodic grep audit. |
| R10 | MCP shell diverges so much it becomes its own framework | Medium | High | MCP is the **contract**; tool surfaces auto-generated. We do not ship LLM-orchestration primitives. ADR-0004 boundary. |
| R11 | **Windows-on-Docker + testcontainers slowness** (3-5× Linux) | **High** | Medium | CI Windows job times out gracefully; `cargo nextest run --test-threads=2` for Windows; document container-startup workarounds in CONTRIBUTING.md. |
| R12 | **`StorageEngine` single-trait original design collapsed to LCD** | — | — | **Resolved**: split into 5 specialist traits + shared `WriteSink<T>` at Phase 1. |
| R13 | **A10 latency/throughput targets unanchored to workload** | — | — | **Resolved**: A11 (renumbered) defines three named workloads, regression gate, no absolute v0.1 targets. |
| R14 | Discovery (0.0.1) tempts copying FinX-XR designs verbatim | Medium | High | Clean-room rule = re-derive; reading is bounded by what fits in working memory; no extended note-taking from FinX-XR sources. |

---

## 11. Verification Steps

V1. `cargo test --workspace --all-features` on Ubuntu + Windows-MSVC every PR. (A1)
V2. `examples/02_storage_roundtrip.rs` writes `OBBject<EquityHistoricalData>` to ClickHouse and Postgres, reads back. CI gates with `testcontainers`. (A2, A4)
V3. `examples/01_basic_fetch.rs` swaps `?provider=fileset` ↔ `?provider=polygon`; identical schema, different bytes. (A3)
V4. `examples/05_streaming_provider.rs` consumes the `Streamer` trait against `tdw-provider-ws-mock`; receives N synthetic events; checkpoint round-trips. (A2)
V5. HTTP smoke: `curl /v1/equity/historical?symbol=AAPL&provider=fileset` → standardized JSON. (A5)
V6. Worker smoke: `docker compose run tdw-worker --once --job polygon_eod_equities ...` → rows in ClickHouse. (A6)
V7. MCP smoke: `tdw-mcp --stdio` → `tools/list` returns registered Fetchers + Streamers; `tools/call` returns matching data + emits `notifications/progress` during the call. (A7, A9)
V8. Hybrid search smoke: ingest 100 research notes → query → Qdrant + Meili + fused top-10 returned with score breakdown. (A8)
V9. **Provider-registration release-profile check**: `cargo build --release --target x86_64-pc-windows-msvc` + run `examples/04_agent_tool_use.rs --list-providers` → all registered providers visible. CI required, blocks merge on failure. (A10)
V10. **Workload bench harness**: `cargo run -p xtask -- bench` runs the three named workloads, writes JSON to `docs/perf-history.md`. CI rejects >20% regression vs rolling-7-day baseline. (A11)
V11. `docs/repo-verdicts.md` PR-reviewed: each verdict has rationale + integration sketch (including SKIPs). (A12)
V12. `docker compose --profile minimal up -d && cargo run --bin tdw-service` succeeds on a clean Windows 11 machine with Docker Desktop / WSL2. (A13)
V13. **Clean-room boundary audit**: grep the workspace for `finx-` and `tesser-` imports; any hit fails CI. (R9)
V14. **Schema spec/code drift gate**: `cargo run -p xtask -- schema-sync` enumerates every `pub struct` in `tdw-domain` and verifies a matching `## Schema` section exists in the corresponding `docs/schemas/{NN_name}.md` with matching field names + types. CI fails on drift. (R2)

---

## 12. References

**OpenBB Platform source paths (architectural inspiration only, no copying)**
- `openbb_platform/core/openbb_core/provider/abstract/{fetcher,provider,query_params,data}.py`
- `openbb_platform/core/openbb_core/provider/standard_models/`
- `openbb_platform/core/openbb_core/app/{command_runner,router}.py`
- `openbb_platform/providers/yfinance/openbb_yfinance/` (example provider)

**Rust ecosystem crates anchoring the design**
- `async-trait`, `tokio`, `serde`, `schemars`, `validator`, `thiserror`, `bytes`
- `axum`, `tonic`, `tower`, `tower-http`
- `sqlx` (Postgres), `clickhouse` (Rust), `qdrant-client`, `meilisearch-sdk`, `aws-sdk-s3`
- `apalis` (or `river`) for workers
- `inventory` (optional, feature-flagged)
- `testcontainers-rs`, `wiremock`, `criterion`, `cargo-nextest`
- MCP: `rmcp` or `mcp-rust-sdk`

**Permitted reading (inspiration-only, no code-copying)**
- `FinX-XR/docs/tasks/openbb-parity-roadmap.md` — what FinX-XR tried
- `references/tesser/tesser-data/` — handler trait patterns
- `FinX-XR/crates/finx-data/{feed,ws_feed,subscription,sequence}.rs` — streaming primitive shape (for `Streamer` trait derivation)

---

## 13. ADR — Architecture Decision Record

### ADR Header
- **Decision**: Build FinX-Finance as a clean-room Rust trading data warehouse at `C:\Users\ReyDa\FinX-Finance\`, using the OpenBB Platform's provider/fetcher pattern translated to Rust, extended with a `Streamer<Q,D>` sibling trait, specialist storage traits (`OlapEngine`, `RelationalEngine`, `VectorEngine`, `LexicalEngine`, `BlobEngine`) sharing a common `WriteSink<T>` for routing fan-out, and four consumer shells (HTTP/gRPC service, Apalis worker, streaming-aware MCP server, CLI) driving one `CommandRunner` runtime.
- **Drivers**:
  1. Heterogeneous data shapes (timeseries, OLTP, blobs, embeddings, lexical) require specialist storage.
  2. Four consumer personas (service, worker, agent, library) must share one fetch implementation, not four.
  3. Windows 11 + Docker Desktop / WSL2 is the user's primary dev environment.
- **Alternatives considered**:
  - **B (event-sourced command bus)**: rejected — over-engineers for non-existent strict-replay requirement, hurts the LLM-agent loop.
  - **C (SQL-only, no provider layer)**: rejected — punts the actual hard problem (heterogeneous provider abstraction) elsewhere; no answer for streaming, blobs, or MCP.
  - **D (integrate into existing FinX-XR)**: rejected by user — this is a clean-room redesign unconstrained by FinX-XR's accumulated decisions and proprietary licensing. The technical merit of D is acknowledged in §0; the rejection is on scope/ownership grounds, not architecture.
- **Why chosen**: Option A is the only one that (a) honors the heterogeneous-shape principle without faking unification, (b) gives one consumer-agnostic orchestrator, (c) is implementable on Windows-MSVC with explicit registration, and (d) stays compatible with the user's existing 11 BOM schemas as the canonical domain.
- **Consequences**:
  - 16-20 crates in a single workspace — discipline required to keep trait shapes honest.
  - Two trait families (`Fetcher`, `Streamer`) shipped together at Phase 1 — frozen API early.
  - Five specialist storage traits — more code than a single `StorageEngine`, but each engine retains its idiomatic shape.
  - `tdw-provider-openbb` deferred to v2 — defending "BOM canonical" requires not muddying v0.1.
  - Windows CI matrix mandatory — Linux-only CI would mask the real-world `inventory` failure mode.
  - No code-sharing with FinX-XR — separate license, separate evolution.
- **Follow-ups**:
  - ADR-0001 boundary rules (clean-room w.r.t. FinX-XR)
  - ADR-0002 Fetcher vs Streamer split rationale
  - ADR-0003 storage trait split rationale
  - ADR-0004 MCP streaming-aware design + error-mapping boundary
  - ADR-0005 explicit vs inventory registration
  - ADR-0006 workload-anchored SLOs (no absolute v0.1 targets)
  - ADR-0007 BOM schema re-derivation provenance (Phase 0.0.2) + spec/code drift gate
  - ADR-0008 Apalis vs RiverQueue choice (Phase 0.1)
  - ADR-0009 license = personal (recorded 2026-05-21)
  - ADR-0010 permanent non-goal: no OpenBB bridge (recorded 2026-05-21)

---

## 14. Open Questions

### Resolved (2026-05-21)
- **O1** ✓ — **BOM schemas re-derived from scratch in Phase 0.0.2.** Two-place layout: markdown specs in `docs/schemas/01_..md` through `11_..md`; Rust structs in `crates/tdw-domain/src/`. CI gate V14 catches drift. Curated OSS-only source list documented in `docs/schemas/00_provenance.md`. No FinX-XR sources in the corpus.
- **O2** ✓ — **License = personal.** Not OSS, not commercial; private codebase for the user. ADR-0009 records this.
- **O3** ✓ — **No `tdw-provider-openbb` bridge, permanent non-goal.** OpenBB is inspiration only. ADR-0001 records this stance.

### Resolved (2026-05-21, continued)
- **O4** ✓ — **RiverQueue** chosen as the worker runtime. ADR-0008 records.
- **O5** ✓ — **Three embedding providers ship at Phase 4**: `tdw-embed-local` (fastembed-rs / candle, default bge-large-en-v1.5), `tdw-embed-openai` (text-embedding-3-large), `tdw-embed-google` (text-embedding-004 / gemini-embedding-001). User picks at setup time via config; Qdrant collections named `{schema}__{model_id}` so they can coexist.

### Still open (no longer blocking — can be resolved later as needed)
- None. All Phase 0 blockers resolved.

---

## 15. Changelog (consensus loop)

**v2 — 2026-05-21 — after Architect + Critic review**:
- Added §0 "Relationship to FinX-XR" with clean-room boundary rules.
- Added §1 RALPLAN-DR summary with Principles, Drivers, Options A/B/C/D + invalidation rationales.
- Promoted `Streamer<Q,D>` to a Phase 1 deliverable (was deferred). New A2/A3 acceptance criteria, `tdw-provider-ws-mock` provider, V4 verification.
- Split `StorageEngine` single trait into five specialist traits + shared `WriteSink<T>`. Updated §4 contracts, §6 storage matrix.
- Added `CommandRunner::run_streaming` + `ProgressOrResult<T>` for streaming-aware MCP. New A9.
- Promoted explicit `register_provider()` to primary registration; `inventory::submit!` to optional feature flag. New A10. Added V9 CI gate on Windows-MSVC release profile.
- Replaced A10's absolute latency numbers with workload-anchored SLOs (now A11). Three named workloads, 20% regression gate, no v0.1 absolute targets.
- Deferred `tdw-provider-openbb` from v0.1 scope to v2.
- Added Phase 0.0 Discovery (before Phase 0.1 Skeleton): BOM schema location, Windows-Docker validation, license decision.
- Added R11 (Windows-on-Docker friction), R12 (storage trait split — resolved), R13 (workload-anchored SLOs — resolved), R14 (FinX-XR copy-temptation).
- Added V13 clean-room boundary audit (grep for `finx-`/`tesser-` imports → CI fail).
- Added §13 ADR with full Decision/Drivers/Alternatives/Why/Consequences/Follow-ups.
- Added §14 Open Questions blocking Phase 0.
- Moved plan to `C:\Users\ReyDa\FinX-Finance\.plans\`.

**v1 — 2026-05-21 — initial direct-mode plan** (saved at `C:\Users\ReyDa\.omc\plans\2026-05-21-rust-trading-data-warehouse.md`).

**v2.1 — 2026-05-21 — all open questions resolved**:
- O1: BOM schemas re-derived from scratch in Phase 0.0.2, two-place layout (`docs/schemas/` markdown + `tdw-domain` Rust structs), CI drift gate V14.
- O2: license = personal.
- O3: no OpenBB bridge — permanent non-goal.
- O4: RiverQueue chosen as worker runtime.
- O5: three embedding providers ship at Phase 4 (`tdw-embed-local`, `tdw-embed-openai`, `tdw-embed-google`); choosable at setup; Qdrant collections named `{schema}__{model_id}`.
- Phase 0.0 extended to 0–2 days to accommodate schema re-synthesis.
- Phase 4 extended to days 14–20 to cover three embedding-provider impls.
- Added V14 (schema spec/code drift gate), ADR-0009 (license = personal), ADR-0010 (no OpenBB bridge).
- R2, R4, R5 marked Resolved.
