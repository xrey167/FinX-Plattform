# FinX-Finance — Data Engineering Layer + Agent Schemas — Plan Extension

**Project:** FinX-Finance (`C:\Users\ReyDa\FinX-Finance\`)
**Date:** 2026-05-21
**Mode:** `/omc-plan --direct` (extension to v2.1)
**Status:** Draft — extends `2026-05-21-rust-trading-data-warehouse.md` with Phases 7 and 8
**Parent plan:** [`2026-05-21-rust-trading-data-warehouse.md`](./2026-05-21-rust-trading-data-warehouse.md)

---

## 1. What this plan adds

The parent plan (v2.1) covers the data **acquisition + storage + serving** layer: Fetchers/Streamers, storage engines, four consumer shells, hybrid retrieval. This extension adds two layers on top:

**Layer A — Data Engineering (Phase 7)**
- SQL transformation files (raw SQL templates + macros)
- dbt models (staging / intermediate / marts, layered medallion architecture)
- ETL/ELT jobs (RiverQueue jobs that orchestrate dbt + raw SQL)
- Analytics schemas (Postgres + ClickHouse logical separation: `raw` / `staging` / `analytics` / `marts`)
- Table definitions (DDL via migration tools, `CREATE TABLE` per domain)

**Layer B — Agent Schemas (Phase 8)**
- Rust schemas for 11 agent-related types, persisted in the warehouse
- Storage mapping (which agent type → which storage engine)
- Authoring formats (Markdown + frontmatter, mirroring Claude Code skills/commands)
- Eval datasets + runs + metrics with full traceability
- Multi-format content schemas (xlsx, csv, pdf, docx, pptx, parquet, json, jsonl)
- Gotcha registry with structured failure-mode taxonomy

These layers are **independent** — Layer A can ship without Layer B, and vice versa. They share storage (Postgres + ClickHouse + Qdrant + Meilisearch) but no code dependencies. Layer B may ship first if the agent infrastructure is more urgent for the user.

---

## 2. Acceptance Criteria

### Layer A — Data Engineering

A7.1. **`dbt/` project** at `C:\Users\ReyDa\FinX-Finance\dbt\finx_finance\` with both `dbt-postgres` and `dbt-clickhouse` profiles configured; `dbt debug` succeeds against the Docker Compose stack.
A7.2. **Medallion layered models** — `models/bronze/` (raw passthroughs), `models/silver/` (cleaned, typed, deduplicated), `models/gold/` (analytics marts joined across domains). Each domain (market_data, fundamentals, news, agents, evals) has at least one bronze + silver + gold model.
A7.3. **At least 10 dbt models** ship at Phase 7 exit: 4 market-data (bronze ohlcv, silver ohlcv adjusted, gold daily_returns, gold rolling_volatility), 2 fundamentals (silver balance_sheet, gold financial_ratios), 2 news (silver news_normalized, gold news_sentiment_panel), 2 agent (silver eval_runs, gold eval_leaderboard).
A7.4. **dbt tests** — every model has `unique`, `not_null` on PK fields; `accepted_values` on enum-typed columns; at least one `dbt-utils` `expression_is_true` per gold model (e.g. "close >= 0").
A7.5. **SQL macros** — at least 3 reusable macros in `dbt/finx_finance/macros/`: `clean_symbol(s)`, `business_day_only(date_col)`, `winsorize(col, p)`.
A7.6. **Migration tooling** — `sqlx-migrate` for Postgres (`migrations/postgres/`); `refinery` or a custom CH migration tool for ClickHouse (`migrations/clickhouse/`); `xtask migrate up/down/status` wraps both.
A7.7. **ETL/ELT job in `tdw-pipeline`** — declarative job that runs `dbt run --select staging.market_data+` after Polygon EOD ingest, with retries and dependency on the prior fetch job. Job dependency DAG is verified by a unit test.
A7.8. **Analytics schema separation** — Postgres has `raw`, `staging`, `analytics`, `marts` schemas with explicit grants (read-only to most consumers). ClickHouse has separate databases `raw`, `staging`, `analytics`, `marts`.
A7.9. **DDL is generated from `tdw-domain`** — `cargo run -p xtask -- ddl-export --target postgres > sql/ddl/postgres_bronze.sql` produces idempotent CREATE TABLE statements derived from the Rust structs. Verified by `dbt run` succeeding against the generated DDL.
A7.10. **Lineage** — every dbt model surfaces source metadata (fetched_at, provider, run_id) via a `_meta` model that joins to RiverQueue job IDs.

### Layer B — Agent Schemas

A8.1. **11 agent types** are defined as Rust schemas in `crates/tdw-agent/src/`: `agent_card.rs`, `agent.rs`, `sub_agent.rs`, `skill.rs`, `tool.rs`, `prompt.rs`, `workflow.rs`, `command.rs`, `steering.rs`, `eval.rs`, `content_schema.rs`, `gotcha.rs`. Each derives `Serialize, Deserialize, JsonSchema, Validate`.
A8.2. **Agent Card** conforms to the A2A (Agent2Agent) Protocol's `agent.json` format — round-trips against the published JSON Schema.
A8.3. **Skill schema** parses Claude Code's `SKILL.md` format (YAML frontmatter + markdown body) via `serde_yaml + pulldown-cmark`. Round-trips a sample skill from the user's existing `.claude/skills/` directory.
A8.4. **Command schema** parses Claude Code's slash-command markdown format with frontmatter.
A8.5. **Content schemas** define typed wrappers for 7 formats: `xlsx`, `csv`, `parquet`, `pdf`, `docx`, `pptx`, `jsonl`. Each has a `ContentRef { storage: BlobRef, content_type, schema_version, … }`.
A8.6. **Eval schema** supports: `EvalDataset` (name, version, items[]), `EvalItem` (input, expected, metadata), `EvalRun` (run_id, dataset_ref, agent_ref, started_at, finished_at, metrics{}), `EvalMetric` (name, value, ci_lo, ci_hi).
A8.7. **Workflow schema** supports DAGs of `WorkflowStep` (id, depends_on[], action, retry_policy, timeout), validated for cycle-freeness.
A8.8. **Storage mapping** is explicit: agent metadata → Postgres (`agent` schema); eval runs → ClickHouse (`marts.eval_runs`); prompt/skill embeddings → Qdrant; lexical search over skills/commands → Meilisearch; raw eval artifacts (transcripts, screenshots) → S3.
A8.9. **Gotcha registry** stores at least 10 entries by Phase 8 exit, each with: trigger pattern, symptom, root cause, mitigation, references (URLs or commit SHAs), severity, last-confirmed date.
A8.10. **Agent runtime hook** — `tdw-mcp` exposes a `tdw.agents.list_skills` and `tdw.agents.search_skills` tool surface backed by Postgres + Meilisearch + Qdrant.
A8.11. **Round-trip parity** — every schema has a `tests/golden/` fixture (real-world example) that parses, re-serializes, and asserts byte-stable JSON (modulo whitespace/key-order normalization).

---

## 3. Layer A — Data Engineering

### 3.1 Directory layout

```
FinX-Finance/
├── dbt/
│   └── finx_finance/                       ← dbt project root
│       ├── dbt_project.yml
│       ├── profiles.yml.template
│       ├── packages.yml                    ← dbt-utils, dbt-expectations
│       ├── models/
│       │   ├── bronze/                     ← raw passthroughs (1:1 with provider output)
│       │   │   ├── market_data/
│       │   │   │   ├── bronze_polygon_ohlcv.sql
│       │   │   │   ├── bronze_yahoo_ohlcv.sql
│       │   │   │   └── bronze_fileset_csv.sql
│       │   │   ├── fundamentals/
│       │   │   │   ├── bronze_sec_edgar_filings.sql
│       │   │   │   └── bronze_fmp_balance_sheet.sql
│       │   │   ├── news/
│       │   │   │   └── bronze_news_items.sql
│       │   │   ├── agents/                 ← bronze for agent runs
│       │   │   │   ├── bronze_agent_runs.sql
│       │   │   │   └── bronze_eval_runs.sql
│       │   │   └── _sources.yml            ← dbt sources for raw tables
│       │   ├── silver/                     ← cleaned, typed, deduped, conformed
│       │   │   ├── market_data/
│       │   │   │   ├── silver_ohlcv_adjusted.sql
│       │   │   │   ├── silver_ohlcv_unified.sql        ← UNION across providers
│       │   │   │   └── silver_corporate_actions.sql
│       │   │   ├── fundamentals/
│       │   │   │   ├── silver_balance_sheet.sql
│       │   │   │   ├── silver_income_statement.sql
│       │   │   │   └── silver_cash_flow.sql
│       │   │   ├── news/
│       │   │   │   └── silver_news_normalized.sql
│       │   │   └── agents/
│       │   │       └── silver_eval_runs.sql
│       │   ├── intermediate/                ← shared joins / window functions
│       │   │   ├── int_returns_daily.sql
│       │   │   ├── int_rolling_stats.sql
│       │   │   └── int_news_sentiment_panel.sql
│       │   ├── gold/                       ← analytics marts
│       │   │   ├── market_data/
│       │   │   │   ├── gold_daily_returns.sql
│       │   │   │   ├── gold_rolling_volatility.sql
│       │   │   │   └── gold_factor_panel.sql
│       │   │   ├── fundamentals/
│       │   │   │   └── gold_financial_ratios.sql
│       │   │   ├── news/
│       │   │   │   └── gold_news_sentiment_panel.sql
│       │   │   └── agents/
│       │   │       └── gold_eval_leaderboard.sql
│       │   └── _meta/                       ← lineage + run metadata
│       │       ├── meta_dbt_runs.sql
│       │       └── meta_source_freshness.sql
│       ├── macros/
│       │   ├── clean_symbol.sql
│       │   ├── business_day_only.sql
│       │   ├── winsorize.sql
│       │   ├── log_return.sql
│       │   └── pit_join.sql                ← point-in-time join for fundamentals
│       ├── seeds/
│       │   ├── ref_exchange_calendar.csv
│       │   ├── ref_country_codes.csv
│       │   └── ref_currency_codes.csv
│       ├── tests/
│       │   ├── generic/                    ← reusable test definitions
│       │   │   ├── ohlcv_invariant_high_ge_low.sql
│       │   │   └── monotonic_timestamps.sql
│       │   └── singular/                   ← table-specific tests
│       └── snapshots/                       ← SCD2 tracking for ref data
│           └── snap_instruments.sql
│
├── sql/
│   ├── ddl/
│   │   ├── postgres/                       ← generated from tdw-domain via xtask
│   │   │   ├── bronze.sql
│   │   │   ├── silver.sql
│   │   │   ├── agents.sql                  ← agent metadata tables
│   │   │   └── grants.sql                  ← schema-level grants
│   │   └── clickhouse/                     ← generated; partitioned + ordered
│   │       ├── bronze.sql                  ← MergeTree definitions
│   │       ├── silver.sql                  ← MaterializedView pipeline (optional)
│   │       └── eval_runs.sql               ← ClickHouse OLAP for eval observability
│   ├── migrations/
│   │   ├── postgres/                       ← sqlx-migrate compatible
│   │   │   ├── 20260521_0001_init_schemas.sql
│   │   │   ├── 20260521_0002_bronze_market_data.sql
│   │   │   ├── 20260521_0003_silver_market_data.sql
│   │   │   ├── 20260521_0004_agents_schema.sql
│   │   │   └── 20260521_0005_eval_artifacts.sql
│   │   └── clickhouse/                     ← refinery-compat or custom runner
│   │       ├── 20260521_0001_init_databases.sql
│   │       ├── 20260521_0002_bronze_ohlcv.sql
│   │       └── 20260521_0003_eval_runs.sql
│   ├── views/                              ← reusable raw SQL views (no dbt)
│   │   └── v_latest_eod_per_symbol.sql
│   └── stored/                             ← Postgres functions / CH UDFs
│       └── pg_fn_ann_volatility.sql
│
└── crates/
    ├── tdw-dbt-runner/                     ← Rust crate that invokes dbt CLI
    │                                          via std::process + parses run_results.json
    ├── tdw-sql-codegen/                    ← derives DDL from tdw-domain
    └── tdw-migration/                      ← wraps sqlx + CH migration runners
```

### 3.2 Storage schemas (Postgres + ClickHouse)

**Postgres schemas:**

| Schema | Purpose | Examples | Grant model |
|--------|---------|----------|-------------|
| `raw`     | Provider passthrough (mirrored by ETL from CH for OLTP-shaped data) | `raw.polygon_ohlcv_landing` | RW: ingest-worker only |
| `staging` | dbt staging models (silver layer) | `staging.silver_balance_sheet` | RW: dbt; R: analytics-consumers |
| `analytics` | dbt gold marts | `analytics.gold_financial_ratios` | RW: dbt; R: api/users |
| `marts`   | curated business marts joined across domains | `marts.daily_factor_panel` | R: everyone |
| `agents`  | agent metadata: agents, skills, tools, prompts, commands, gotchas | `agents.agent`, `agents.skill` | RW: tdw-agent; R: api/mcp |
| `evals`   | eval datasets metadata (runs go to ClickHouse) | `evals.dataset`, `evals.item` | RW: eval-runner; R: api |
| `system`  | internal: job queue (RiverQueue), watermarks, lineage | `system.river_job` | RW: workers |

**ClickHouse databases:**

| Database | Purpose | Storage engine | Example table |
|----------|---------|----------------|---------------|
| `raw`         | Tick + bar wide-row landing | `MergeTree` partitioned by `toYYYYMM(ts)`, ordered by `(symbol, ts)` | `raw.polygon_trades` |
| `staging`     | Cleaned silver (optional — many users skip CH silver, only mart gold) | `MergeTree` | `staging.silver_ohlcv` |
| `analytics`   | Gold marts (timeseries shapes) | `MergeTree` + `MaterializedView` | `analytics.gold_daily_returns` |
| `marts`       | Cross-domain wide tables for dashboards | `MergeTree` | `marts.factor_panel_daily` |
| `evals`       | Eval observability: every step of every agent run, with tracing | `MergeTree` partitioned by `toYYYYMM(started_at)`, ordered by `(run_id, step_seq)` | `evals.run_step` |
| `events`      | All agent + ingest + worker events for observability/auditing | `MergeTree` | `events.system_event` |

### 3.3 dbt configuration

`dbt_project.yml` highlights:
```yaml
name: 'finx_finance'
version: '0.1.0'
config-version: 2

profile: 'finx_finance'           # uses dbt-postgres OR dbt-clickhouse depending on target

models:
  finx_finance:
    bronze:
      +materialized: view
      +schema: bronze
      +tags: ['layer:bronze']
    silver:
      +materialized: table         # silver materialized for query speed
      +schema: staging
      +tags: ['layer:silver']
    intermediate:
      +materialized: ephemeral     # in-CTE, not persisted
      +tags: ['layer:intermediate']
    gold:
      +materialized: incremental
      +incremental_strategy: merge
      +schema: analytics
      +tags: ['layer:gold']
    _meta:
      +materialized: view
      +schema: system
```

Two `profiles.yml` targets:
```yaml
finx_finance:
  outputs:
    pg_dev:
      type: postgres
      host: localhost
      port: 5432
      user: dbt
      password: "{{ env_var('DBT_PG_PWD') }}"
      dbname: finx
      schema: analytics
      threads: 8
    ch_dev:
      type: clickhouse
      driver: native
      host: localhost
      port: 9000
      user: dbt
      password: "{{ env_var('DBT_CH_PWD') }}"
      schema: analytics
      threads: 8
  target: pg_dev
```

**Dispatch rule**: Postgres-shaped data (reference, fundamentals, accounts, agent metadata) uses `pg_dev`; ClickHouse-shaped data (OHLCV, ticks, eval observability) uses `ch_dev`. dbt models declare their target via `{{ config(target='ch_dev') }}` or the project-level dispatch macro.

### 3.4 ETL/ELT orchestration (RiverQueue → dbt)

ETL jobs are RiverQueue jobs. Each job has a declarative dependency graph:

```toml
# C:\Users\ReyDa\FinX-Finance\config\pipelines\market_data_eod.toml

[[job]]
name      = "polygon_eod_ingest"
schedule  = "0 22 * * 1-5"                              # weekdays 22:00 UTC
fetcher   = "polygon::equity_historical"
params    = { symbols_from = "system.active_symbols" }
storage   = ["clickhouse:raw.polygon_ohlcv_landing"]
retries   = 3
backoff   = "exponential(base=5s, max=2m)"

[[job]]
name        = "dbt_bronze_market_data"
depends_on  = ["polygon_eod_ingest"]
runner      = "dbt"
dbt_args    = "run --select tag:layer:bronze tag:domain:market_data"
target      = "ch_dev"

[[job]]
name        = "dbt_silver_market_data"
depends_on  = ["dbt_bronze_market_data"]
runner      = "dbt"
dbt_args    = "run --select tag:layer:silver tag:domain:market_data"

[[job]]
name        = "dbt_gold_market_data"
depends_on  = ["dbt_silver_market_data"]
runner      = "dbt"
dbt_args    = "run --select tag:layer:gold tag:domain:market_data"

[[job]]
name        = "dbt_test_market_data"
depends_on  = ["dbt_gold_market_data"]
runner      = "dbt"
dbt_args    = "test --select tag:domain:market_data"
```

**Runner abstraction:** `tdw-dbt-runner` invokes the dbt CLI via `std::process::Command`, captures `target/run_results.json`, parses it into typed Rust structs (`DbtRunResult`, `DbtNodeResult`), and emits one ClickHouse `evals.run_step` row per dbt node so dbt runs are first-class observable in the eval/observability layer.

### 3.5 DDL generation from `tdw-domain`

The Rust structs in `tdw-domain` are the canonical schema. `tdw-sql-codegen` walks them and emits idempotent DDL:

```rust
// crates/tdw-sql-codegen/src/lib.rs

pub trait SqlSchema {
    fn table_name() -> &'static str;
    fn create_table_postgres() -> String;
    fn create_table_clickhouse() -> String;
}

#[derive(SqlSchema)]                              // derive macro
#[sql(
    pg_schema = "bronze",
    pg_pk = "symbol, ts",
    ch_db = "raw",
    ch_order_by = "(symbol, ts)",
    ch_partition_by = "toYYYYMM(ts)"
)]
pub struct EquityHistoricalData {
    pub symbol: String,
    pub ts: DateTime<Utc>,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: u64,
    pub provider: String,
    pub fetched_at: DateTime<Utc>,
}
```

Generated DDL:
```sql
-- Postgres
CREATE TABLE IF NOT EXISTS bronze.equity_historical (
    symbol      TEXT        NOT NULL,
    ts          TIMESTAMPTZ NOT NULL,
    open        DOUBLE PRECISION NOT NULL,
    high        DOUBLE PRECISION NOT NULL,
    low         DOUBLE PRECISION NOT NULL,
    close       DOUBLE PRECISION NOT NULL,
    volume      BIGINT      NOT NULL,
    provider    TEXT        NOT NULL,
    fetched_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (symbol, ts)
);

-- ClickHouse
CREATE TABLE IF NOT EXISTS raw.equity_historical (
    symbol      String,
    ts          DateTime64(3, 'UTC'),
    open        Float64,
    high        Float64,
    low         Float64,
    close       Float64,
    volume      UInt64,
    provider    LowCardinality(String),
    fetched_at  DateTime64(3, 'UTC') DEFAULT now64()
) ENGINE = MergeTree
PARTITION BY toYYYYMM(ts)
ORDER BY (symbol, ts);
```

Single source of truth = Rust. dbt sources reference these tables. V9 (extended) verifies the DDL via golden snapshot tests.

---

## 4. Layer B — Agent Schemas

### 4.1 The 11 agent schema types

Single crate `tdw-agent` with submodules — easier dependency story than 11 crates. Each module is one file with structs + tests.

```
crates/tdw-agent/src/
├── lib.rs                       ← re-exports + the Registry trait
├── agent_card.rs                ← A2A protocol agent.json
├── agent.rs                     ← Agent definition
├── sub_agent.rs                 ← Hierarchical sub-agent
├── skill.rs                     ← Claude Code SKILL.md parser
├── tool.rs                      ← Tool spec (MCP-compatible)
├── prompt.rs                    ← Prompt template
├── workflow.rs                  ← Workflow DAG
├── command.rs                   ← Slash command spec
├── steering.rs                  ← Steering rules
├── eval.rs                      ← EvalDataset, EvalRun, EvalMetric
├── content_schema.rs            ← xlsx, csv, pdf, docx, pptx, parquet, jsonl
└── gotcha.rs                    ← Gotcha registry entries
```

#### 4.1.1 `Agent Card` (`agent_card.rs`)

Conforms to A2A (Agent2Agent) Protocol's `agent.json` format — published by Google as an open protocol for agent discovery and capability advertisement.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct AgentCard {
    /// Schema version (e.g., "1.0").
    pub spec_version: String,
    /// Unique agent identifier (URI-shaped, e.g., "https://example.com/agents/research-bot").
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// One-line description.
    pub description: String,
    /// Long-form description (markdown).
    pub long_description: Option<String>,
    /// Service endpoint URL.
    pub url: Url,
    /// Authentication schemes supported.
    pub authentication: Vec<AuthScheme>,
    /// Skill descriptors (high-level capabilities).
    pub skills: Vec<AgentCardSkill>,
    /// Supported input modalities (e.g. "text", "image", "audio").
    pub input_modalities: Vec<Modality>,
    /// Supported output modalities.
    pub output_modalities: Vec<Modality>,
    /// Provider metadata (name, version, contact).
    pub provider: AgentProvider,
    /// Optional extensions (k/v).
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub enum AuthScheme {
    None,
    BearerToken,
    OAuth2 { authorize_url: Url, token_url: Url, scopes: Vec<String> },
    ApiKey { header_name: String },
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AgentCardSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub examples: Vec<String>,
    pub input_schema: Option<serde_json::Value>,    // JSON Schema
    pub output_schema: Option<serde_json::Value>,
}
```

**Storage**: Postgres `agents.agent_card` (one row per agent). The full JSON is stored as `JSONB` for fidelity; key fields are extracted as columns for indexing.

#### 4.1.2 `Agent` (`agent.rs`)

The runtime agent definition — points at a model, system prompt, tool set, steering rules, and an optional sub-agent topology.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct Agent {
    pub id: String,                                  // ULID
    pub name: String,
    pub description: String,
    pub version: SemVer,
    pub model: ModelRef,                             // "claude-opus-4-7", "gpt-4o", "gemini-2.0-pro", "local:llama-3.3-70b"
    pub system_prompt: PromptRef,                    // FK to prompt.rs
    pub tools: Vec<ToolRef>,                         // FKs to tool.rs
    pub steerings: Vec<SteeringRef>,                 // FKs to steering.rs
    pub sub_agents: Vec<SubAgentRef>,                // FKs to sub_agent.rs
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
    pub agent_card: Option<AgentCardRef>,            // optional public-facing card
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ModelRef {
    pub provider: ModelProvider,                     // Anthropic, OpenAI, Google, Local
    pub model_id: String,                            // "claude-opus-4-7"
    pub api_version: Option<String>,
    pub fallback: Option<Box<ModelRef>>,             // tower-of-models fallback
}
```

**Storage**: Postgres `agents.agent` (one row per agent), with FKs to tools/prompts/steerings tables.

#### 4.1.3 `SubAgent` (`sub_agent.rs`)

A sub-agent is an `Agent` that can be invoked by a parent agent through a typed handoff. The schema captures handoff semantics (input/output shape, exit conditions, retry policy).

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct SubAgent {
    pub id: String,
    pub agent_id: AgentRef,                          // points at an Agent row
    pub parent_id: AgentRef,                         // parent that can invoke this
    pub trigger: SubAgentTrigger,                    // tool_call | keyword | always
    pub handoff_input_schema: serde_json::Value,     // JSON Schema for input
    pub handoff_output_schema: serde_json::Value,
    pub exit_condition: ExitCondition,               // max_turns | tool_call("done") | regex(...)
    pub return_to_parent: bool,                      // false = sub-agent terminates run
    pub retry_policy: RetryPolicy,
    pub max_runtime_seconds: u32,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub enum SubAgentTrigger {
    ToolCall(String),                                // parent calls tool "research" → routes to this sub-agent
    Keyword { patterns: Vec<String> },               // parent message matches any pattern
    Always,                                          // every turn fans out to this sub-agent
    Manual,                                          // user-triggered only
}
```

**Storage**: Postgres `agents.sub_agent` (composite PK: agent_id, parent_id).

#### 4.1.4 `Skill` (`skill.rs`)

Parses Claude Code's `SKILL.md` format — YAML frontmatter + markdown body. The parser is symmetric: read existing skills from `~/.claude/skills/`, write new ones.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct Skill {
    // Frontmatter
    pub name: String,                                // unique slug
    pub description: String,                         // triggers when keywords match this
    pub when_to_use: Vec<String>,                    // explicit invocation contexts
    pub allowed_tools: Vec<ToolRef>,                 // tools this skill can call
    pub forbidden_tools: Vec<ToolRef>,
    pub argument_hint: Option<String>,
    pub model_hint: Option<ModelRef>,                // "use opus for this"
    pub base_directory: Option<PathBuf>,             // where skill resources live
    pub examples: Vec<SkillExample>,
    // Body
    pub body_markdown: String,                       // the SKILL.md prose
    pub body_html: Option<String>,                   // cached render
    // Provenance
    pub file_path: Option<PathBuf>,
    pub author: Option<String>,
    pub version: SemVer,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Skill {
    pub fn parse_from_markdown(input: &str) -> Result<Skill> { /* serde_yaml + pulldown-cmark */ }
    pub fn render_to_markdown(&self) -> String { /* round-trip */ }
}
```

**Storage**:
- Postgres `agents.skill` (metadata + frontmatter columns extracted).
- S3 (`agents/skills/{name}/{version}.md`) for the canonical markdown file.
- Qdrant collection `agent_skills__{embed_model}` for semantic search over `description + body_markdown`.
- Meilisearch index `agent_skills` for lexical search.

A `tdw-mcp` tool `tdw.agents.search_skills(query, top_k)` fans out to Qdrant + Meili and returns hybrid-ranked results — equivalent to the `research_note` hybrid retrieval but over the agent skill corpus.

#### 4.1.5 `Tool` (`tool.rs`)

Tools are MCP-compatible by default (so `tdw-mcp` can re-emit them directly), but stored independently in case agents use non-MCP tools (function-call schemas, OpenAI function format, etc.).

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,             // JSON Schema for inputs
    pub output_schema: Option<serde_json::Value>,
    pub kind: ToolKind,                              // McpTool | OpenAiFunction | AnthropicTool | Internal
    pub implementation: ToolImpl,                    // McpServer { url } | RustFn { crate, fn_path } | HttpEndpoint
    pub side_effects: SideEffects,                   // ReadOnly | WriteSafe | Destructive
    pub auth_required: bool,
    pub rate_limit: Option<RateLimit>,
    pub gotchas: Vec<GotchaRef>,                     // known failure modes
    pub examples: Vec<ToolExample>,
    pub version: SemVer,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub enum SideEffects {
    ReadOnly,
    WriteSafe,                                       // idempotent writes
    Destructive,                                     // requires confirmation
}
```

**Storage**: Postgres `agents.tool`.

#### 4.1.6 `Prompt` (`prompt.rs`)

Prompt templates — Jinja-style (`{{ var }}`, `{% for %}`) via the `minijinja` crate. Templates are versioned; renders are reproducible from `(template_id, variables_hash)`.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct Prompt {
    pub id: String,
    pub name: String,
    pub kind: PromptKind,                            // System | User | Assistant | Tool
    pub template: String,                            // minijinja
    pub variables_schema: serde_json::Value,         // JSON Schema for variables
    pub language: String,                            // "en", "de", …
    pub model_hints: Vec<ModelRef>,                  // models this is tuned for
    pub version: SemVer,
    pub examples: Vec<PromptExample>,                // input vars → expected rendered output
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

impl Prompt {
    pub fn render(&self, vars: serde_json::Value) -> Result<String> {
        let env = minijinja::Environment::new();
        env.render_str(&self.template, vars)
    }
}
```

**Storage**: Postgres `agents.prompt`; Qdrant `agent_prompts__{embed_model}` for semantic search.

#### 4.1.7 `Workflow` (`workflow.rs`)

Multi-step agent workflows as a DAG. Steps can be agent invocations, tool calls, conditional branches, or fan-out/fan-in.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: SemVer,
    pub steps: Vec<WorkflowStep>,
    pub entrypoint: StepId,
    pub timeout_seconds: u32,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WorkflowStep {
    pub id: StepId,
    pub kind: StepKind,                              // AgentInvocation | ToolCall | Conditional | FanOut | FanIn | Wait
    pub depends_on: Vec<StepId>,
    pub action: serde_json::Value,                   // shape depends on kind
    pub retry_policy: RetryPolicy,
    pub timeout_seconds: Option<u32>,
    pub on_failure: FailureAction,                   // Abort | Continue | Compensate(StepId)
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub enum StepKind {
    AgentInvocation { agent_id: AgentRef },
    ToolCall { tool_id: ToolRef },
    Conditional { condition_jsonpath: String, true_branch: StepId, false_branch: StepId },
    FanOut { branches: Vec<StepId>, mode: FanOutMode },
    FanIn { from: Vec<StepId>, reducer: Reducer },
    Wait { duration_seconds: u32 },
    Human { prompt: String },                        // human-in-the-loop checkpoint
}
```

Cycle-freeness is verified on insert via Kahn's topological sort; rejection produces a precise error pointing at the offending edge.

**Storage**: Postgres `agents.workflow` + `agents.workflow_step` (1:N).

#### 4.1.8 `Command` (`command.rs`)

Claude Code-style slash commands. Markdown file with frontmatter; arguments parsed per spec.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct Command {
    pub name: String,                                // "/omc-plan"
    pub description: String,
    pub argument_hint: Option<String>,
    pub allowed_tools: Vec<ToolRef>,
    pub argument_schema: Option<serde_json::Value>,
    pub model_hint: Option<ModelRef>,
    pub body_markdown: String,                       // command instructions
    pub examples: Vec<CommandExample>,
    pub plugin: Option<String>,                      // "oh-my-claudecode"
    pub file_path: Option<PathBuf>,
    pub version: SemVer,
}
```

**Storage**: Postgres `agents.command`; markdown bodies in S3.

#### 4.1.9 `Steering` (`steering.rs`)

Steering rules — behavior modifiers applied to one or more agents. (e.g., "always speak German with this user", "no emojis", "verify before claiming completion".)

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct Steering {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: SteeringKind,                          // BehaviorRule | StyleGuide | ConstraintCheck | Memory
    pub directive: String,                           // the rule text
    pub scope: SteeringScope,                        // Global | PerAgent(AgentRef) | PerUser(UserRef) | PerWorkflow(WorkflowRef)
    pub priority: u8,                                // 0-255, higher = applied first
    pub conditional: Option<String>,                 // jsonpath expression that gates application
    pub created_at: DateTime<Utc>,
    pub last_applied_at: Option<DateTime<Utc>>,
    pub apply_count: u64,                            // observability: how often is this triggered
}
```

**Storage**: Postgres `agents.steering`.

#### 4.1.10 `Eval` (`eval.rs`)

Three interrelated types: dataset, run, metric.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct EvalDataset {
    pub id: String,
    pub name: String,
    pub version: SemVer,
    pub description: String,
    pub items: Vec<EvalItem>,                        // or a pointer to S3 / Parquet
    pub item_count: u32,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EvalItem {
    pub id: String,
    pub input: serde_json::Value,
    pub expected: Option<serde_json::Value>,         // for reference-based metrics
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub weight: f32,                                 // for weighted aggregates
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EvalRun {
    pub id: String,                                  // ULID
    pub dataset_id: EvalDatasetRef,
    pub dataset_version: SemVer,
    pub agent_id: AgentRef,
    pub agent_version: SemVer,
    pub workflow_id: Option<WorkflowRef>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: EvalRunStatus,                       // Pending | Running | Completed | Failed | Cancelled
    pub metrics: Vec<EvalMetric>,
    pub item_results: Vec<EvalItemResult>,            // → S3 for large runs
    pub config_snapshot: serde_json::Value,          // agent config at run time, for reproducibility
    pub git_sha: Option<String>,
    pub cost_usd: Option<f64>,
    pub total_tokens_in: Option<u64>,
    pub total_tokens_out: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EvalMetric {
    pub name: String,                                // "accuracy" | "f1" | "rouge_l" | "judge_score"
    pub value: f64,
    pub ci_lo: Option<f64>,
    pub ci_hi: Option<f64>,
    pub ci_method: Option<String>,                   // "bootstrap_1k"
    pub n_items: u32,
    pub aggregation: Aggregation,                    // Mean | Median | Sum | P50 | P95 | Custom(String)
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EvalItemResult {
    pub item_id: String,
    pub output: serde_json::Value,
    pub passed: Option<bool>,
    pub score: Option<f64>,
    pub latency_ms: u32,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub cost_usd: Option<f64>,
    pub error: Option<String>,
    pub trace_ref: Option<TraceRef>,                 // pointer to OpenTelemetry trace
}
```

**Storage**:
- Postgres `evals.dataset`, `evals.item` (metadata only; large datasets stored in Parquet on S3 with a reference).
- **ClickHouse `evals.run` + `evals.run_step` + `evals.run_item`** — eval runs are observability data, naturally fits CH for fast aggregate queries on the leaderboard mart.
- S3 `evals/{run_id}/items.parquet` for raw item results when run is large.
- dbt model `gold_eval_leaderboard.sql` aggregates by `(agent_id, dataset_id, metric_name)` and surfaces the latest run per agent.

#### 4.1.11 `ContentSchema` (`content_schema.rs`)

Typed wrappers for the content formats agents and skills produce or consume.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct ContentRef {
    pub storage: BlobRef,                            // S3 key + bucket
    pub content_type: ContentType,                   // Xlsx | Csv | Parquet | Pdf | Docx | Pptx | Jsonl | Json | Markdown
    pub schema_version: SemVer,
    pub size_bytes: u64,
    pub checksum_sha256: String,
    pub created_at: DateTime<Utc>,
    pub origin: ContentOrigin,                       // AgentOutput | UserUpload | DataExport | SkillTemplate
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub enum ContentType {
    Xlsx { sheets: Vec<XlsxSheetSchema> },
    Csv { delimiter: char, has_header: bool, columns: Vec<CsvColumn> },
    Parquet { schema: ParquetSchemaJson },
    Pdf { page_count: u32, ocr_available: bool },
    Docx { word_count: Option<u32> },
    Pptx { slide_count: u32 },
    Jsonl { record_schema: serde_json::Value, line_count: u64 },
    Json { schema: serde_json::Value },
    Markdown { has_frontmatter: bool },
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct XlsxSheetSchema {
    pub name: String,
    pub row_count: u32,
    pub columns: Vec<XlsxColumn>,
    pub named_ranges: Vec<XlsxNamedRange>,
    pub has_formulas: bool,
    pub frozen_rows: u32,
    pub frozen_cols: u32,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct XlsxColumn {
    pub letter: String,                              // "A", "B"
    pub header: Option<String>,                      // from row 1 if present
    pub inferred_type: ContentColumnType,            // String | Integer | Float | Date | DateTime | Bool | Mixed
    pub null_fraction: f32,
}
```

**Why this matters**: skills like `pitch-deck`, `xlsx-author`, `audit-xls`, `3-statement-model` all produce or consume xlsx/pptx/csv files. Typed schemas at this layer make skill outputs introspectable, indexable, and joinable to provenance (which agent run produced this file? which dataset row did it consume?).

**Storage**: Postgres `agents.content_ref` (metadata); raw bytes in S3.

#### 4.1.12 `Gotcha` (`gotcha.rs`)

Structured failure-mode registry. Every gotcha is a unit of organizational learning.

```rust
#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct Gotcha {
    pub id: String,                                  // ULID
    pub title: String,                               // one-line summary
    pub category: GotchaCategory,                    // ToolUsage | ModelBehavior | DataQuality | Permission | RateLimit | Cost | Hallucination | InfraFlakiness
    pub severity: Severity,                          // Info | Warning | High | Critical
    pub trigger_patterns: Vec<TriggerPattern>,       // regex or jsonpath on agent transcript / tool I/O
    pub symptom: String,                             // what the user / system sees
    pub root_cause: String,                          // why it happens
    pub mitigation: String,                          // what to do about it
    pub workaround: Option<String>,                  // short-term fix
    pub references: Vec<Reference>,                  // URLs, commit SHAs, eval run IDs
    pub affects: Vec<GotchaAffects>,                 // Agent(id) | Tool(id) | Model(name) | Workflow(id) | Global
    pub first_seen_at: DateTime<Utc>,
    pub last_confirmed_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub occurrences: u32,
    pub status: GotchaStatus,                        // Active | Mitigated | Resolved | Stale
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub enum TriggerPattern {
    TranscriptRegex(String),                         // pattern in model output
    ToolErrorContains(String),                       // pattern in tool error message
    LatencyExceeds { ms: u32 },                      // perf-induced
    TokenUsageExceeds { tokens: u32 },               // cost-induced
    JsonPath { path: String, equals: serde_json::Value },
}
```

**Storage**: Postgres `agents.gotcha`; observability events that match a `TriggerPattern` increment `occurrences` and update `last_seen_at` automatically (CH materialized view → Postgres upsert via a small reactor job).

### 4.2 Storage layout (agent schemas)

| Type | Postgres | ClickHouse | Qdrant | Meilisearch | S3 |
|------|----------|------------|--------|-------------|----|
| AgentCard | `agents.agent_card` | — | — | `agent_cards` | — |
| Agent | `agents.agent` | — | — | `agents` | — |
| SubAgent | `agents.sub_agent` | — | — | — | — |
| Skill | `agents.skill` (metadata) | — | `agent_skills__{model}` | `agent_skills` | `agents/skills/{name}/{version}.md` |
| Tool | `agents.tool` | — | — | `agent_tools` | — |
| Prompt | `agents.prompt` | — | `agent_prompts__{model}` | `agent_prompts` | — |
| Workflow | `agents.workflow` + `agents.workflow_step` | — | — | — | — |
| Command | `agents.command` (metadata) | — | — | `agent_commands` | `agents/commands/{name}/{version}.md` |
| Steering | `agents.steering` | — | — | — | — |
| EvalDataset | `evals.dataset`, `evals.item` | — | — | — | `evals/datasets/{id}/v{v}.parquet` |
| EvalRun | — | `evals.run`, `evals.run_step`, `evals.run_item` | — | — | `evals/{run_id}/items.parquet`, `evals/{run_id}/traces/` |
| ContentRef | `agents.content_ref` | — | — | — | raw bytes |
| Gotcha | `agents.gotcha` | `events.gotcha_trigger` (observability) | — | `agent_gotchas` | — |

### 4.3 MCP tool surface for agents

`tdw-mcp` exposes a new tool family `tdw.agents.*`:

| Tool | Purpose |
|------|---------|
| `tdw.agents.list` | List registered agents (paginated, filterable by tag). |
| `tdw.agents.get` | Fetch full agent definition by id. |
| `tdw.agents.search_skills` | Hybrid Qdrant + Meilisearch search over agent skills. |
| `tdw.agents.search_prompts` | Hybrid search over prompt templates. |
| `tdw.agents.render_prompt` | Render a prompt template with variables. |
| `tdw.agents.lookup_gotchas` | Given an agent / tool / model, return active gotchas. |
| `tdw.agents.report_gotcha` | Report a new gotcha occurrence (writes to Postgres + CH). |
| `tdw.agents.eval_run_status` | Get status of an eval run. |
| `tdw.agents.eval_leaderboard` | Query the gold leaderboard mart. |
| `tdw.agents.workflows.run` | Trigger a workflow; returns run id; progress emitted via MCP `notifications/progress`. |

All tools are streaming-aware via the `CommandRunner::run_streaming` path established in v2.1.

---

## 5. New crates added to the workspace

```diff
crates/
  ...
+ tdw-dbt-runner/                 ← invokes dbt CLI, parses run_results.json
+ tdw-sql-codegen/                ← derives DDL from tdw-domain
+ tdw-migration/                  ← sqlx + CH migration wrapper
+ tdw-agent/                      ← all 11 agent schemas (one crate, submodules)
+ tdw-agent-store/                ← persistence: Postgres + Qdrant + Meili adapters for tdw-agent
+ tdw-eval-runner/                ← executes EvalDataset against Agent; writes EvalRun to CH
+ tdw-workflow-engine/            ← executes Workflow DAGs as RiverQueue jobs
```

Updated workspace member count: parent plan (~22) + 7 new = **~29 crates**.

---

## 6. Phase plan (additions to parent v2.1)

### Phase 7 — Data Engineering (days 31–40, after Phase 6)

7.1. Set up `dbt/finx_finance/` project + `profiles.yml.template` + `packages.yml` (dbt-utils, dbt-expectations). Verify `dbt debug` against `docker compose --profile minimal`.
7.2. Author Postgres + ClickHouse migrations for `raw`, `staging`, `analytics`, `marts`, `agents`, `evals`, `system` schemas. Run via `xtask migrate up`.
7.3. Implement `tdw-sql-codegen` derive macro. Generate DDL for the 11 BOM-schema structs. Commit generated SQL under `sql/ddl/` for review.
7.4. Build bronze models (one per active provider × domain): 6 minimum.
7.5. Build silver models with conform + dedupe logic. 4 minimum.
7.6. Build gold marts: 4 minimum (daily_returns, rolling_volatility, financial_ratios, news_sentiment_panel).
7.7. Add dbt tests (unique, not_null, accepted_values, expression_is_true) per A7.4.
7.8. Author macros: `clean_symbol`, `business_day_only`, `winsorize`, `log_return`, `pit_join`.
7.9. Implement `tdw-dbt-runner` — invoke dbt CLI via std::process; parse `target/run_results.json`; emit one CH `evals.run_step` row per node.
7.10. Wire dbt jobs into `tdw-pipeline` with declarative dependency graphs (TOML).
7.11. **Lineage**: `meta_dbt_runs.sql` + `meta_source_freshness.sql` join dbt outputs to RiverQueue job IDs.
7.12. Documentation: `docs/dbt-guide.md`, `docs/sql-conventions.md`, ADR-0011 (dbt-postgres + dbt-clickhouse dispatch rule).

**Exit criteria**: A7.1–A7.10 satisfied.

### Phase 8 — Agent Schemas (days 41–47)

8.1. `tdw-agent` crate with 12 module files (one per schema). All derive `Serialize, Deserialize, JsonSchema, Validate`.
8.2. SKILL.md parser (`Skill::parse_from_markdown` / `render_to_markdown`); round-trip test against 3 fixture skills from `~/.claude/skills/`.
8.3. Slash-command parser; round-trip against 3 fixture commands.
8.4. JSON Schema export: `cargo run -p tdw-agent -- emit-schemas > docs/agent-schemas/`. Used by external editors / linters.
8.5. Postgres migrations for `agents.*` and `evals.*` tables.
8.6. ClickHouse migrations for `evals.run`, `evals.run_step`, `evals.run_item`, `events.gotcha_trigger`.
8.7. `tdw-agent-store`: Postgres CRUD via sqlx; Qdrant collection setup with one collection per `(schema, embed_model)` pair; Meilisearch index setup with searchable attributes derived from struct attributes.
8.8. `tdw-eval-runner` MVP: load `EvalDataset` from S3/Postgres; run an `Agent` over each item; record `EvalRun` + `EvalItemResult[]` to CH + S3.
8.9. `tdw-workflow-engine` MVP: load `Workflow`; validate DAG; compile to RiverQueue jobs; execute; emit progress events.
8.10. `tdw-mcp` new tools (§4.3) — `tdw.agents.list/get/search_skills/search_prompts/render_prompt/lookup_gotchas/report_gotcha/eval_run_status/eval_leaderboard/workflows.run`.
8.11. Seed gotcha registry with 10 entries from the user's existing experience (e.g., "MCP tool hangs on prompt > N tokens", "ClickHouse insert silently truncates strings > LowCardinality cardinality", "Apalis Postgres job table contention under high concurrency", etc.).
8.12. dbt model `gold_eval_leaderboard.sql` aggregates eval runs by `(agent_id, dataset_id, metric_name, week)` and exposes the leaderboard view to MCP.
8.13. Documentation: `docs/agent-schemas-guide.md`, `docs/skill-authoring.md`, ADR-0012 (one crate vs many for agent schemas), ADR-0013 (CH for eval runs + Postgres for metadata).

**Exit criteria**: A8.1–A8.11 satisfied.

---

## 7. Risks & Mitigations

| #    | Risk | Likelihood | Impact | Mitigation |
|------|------|-----------|--------|------------|
| R15  | dbt-postgres + dbt-clickhouse profile drift — same model behaves differently | Medium | Medium | Project-level dispatch macro; per-model target tag; CI runs `dbt build` against both targets for cross-target models. |
| R16  | DDL codegen from Rust diverges from dbt source declarations | Medium | High | V14 (parent plan) extended: schema-sync gate also verifies dbt source declarations match the generated DDL. |
| R17  | dbt run latency dominates the EOD pipeline window | Medium | Medium | `dbt build --select state:modified+` (incremental); use ClickHouse `MaterializedView` for derived tables that don't change shape. |
| R18  | Agent schema sprawl — 12 module files become a maze | Medium | Medium | One crate (not 12); explicit re-exports in `lib.rs`; ADR-0012 records the boundary. |
| R19  | SKILL.md format drifts as Claude Code evolves; parser breaks on new fields | Medium | Medium | Permissive parser: unknown frontmatter fields land in `extensions: BTreeMap<String, serde_json::Value>`; round-trip preserves them. |
| R20  | Storing every eval `run_item` in CH explodes table sizes | High | Medium | Large datasets (>10k items per run) write `EvalItemResult[]` to S3 as Parquet; CH stores only the run header + summary stats. Threshold in config. |
| R21  | Gotcha registry decays — entries grow stale, mitigations get incorrect | High | Medium | `last_confirmed_at` field + nightly job that pings each gotcha's trigger pattern against recent observability events; auto-marks `Stale` after 90 days without a re-confirm. |
| R22  | Workflow DAG cycles slip through static validation when conditional steps depend on runtime data | Medium | High | Static cycle check on insert; runtime guard rails: max step count, max wall-clock, max iterations per branch. Workflow engine refuses to execute a graph with >256 nodes or >5 minutes total without an explicit `large_workflow=true` flag. |
| R23  | dbt + ClickHouse known issues with INSERT-only `incremental_strategy='merge'` (CH doesn't natively support MERGE) | Medium | Medium | Use `incremental_strategy='delete+insert'` with explicit `unique_key`; document the CH-specific surgery in dbt model SQL. |
| R24  | Embedding cost spikes when skill/prompt corpora grow (re-embedding on every change) | Medium | Medium | Hash-based diff: only re-embed when `body_markdown` hash changes; store hash in Postgres alongside the row. |
| R25  | `tdw-eval-runner` running against external LLM APIs racks up cost during dev | High | Medium | Per-dataset cost budget in `EvalDataset.metadata.budget_usd`; runner aborts before exceeding; CI uses fixture/replay mode. |

---

## 8. Verification Steps

### Layer A
V15. `dbt debug` succeeds against both targets in CI. (A7.1)
V16. `dbt build` runs all bronze+silver+gold models in CI against `testcontainers` Postgres + ClickHouse; > 80% test pass rate. (A7.2–A7.4)
V17. SQL macros tested via `dbt run-operation` unit tests under `dbt/finx_finance/tests/macros/`. (A7.5)
V18. Migrations idempotent: `xtask migrate up && xtask migrate up` succeeds twice in a row with no DDL changes. (A7.6)
V19. `tdw-pipeline` job DAG: unit test asserts that `dbt_silver_market_data` is enqueued only after `dbt_bronze_market_data` completes. (A7.7)
V20. Grant audit: `xtask db-audit-grants` reports the expected grants per schema; CI fails on drift. (A7.8)
V21. DDL codegen roundtrip: `xtask ddl-export --dry-run` produces zero-diff against `sql/ddl/` in CI. (A7.9)
V22. `meta_source_freshness` returns < 25h staleness for every active source. (A7.10)

### Layer B
V23. JSON Schema export of every agent schema validates against itself and against fixtures under `tests/golden/`. (A8.1)
V24. `Skill::parse_from_markdown` roundtrips 3 user fixture skills byte-stable. (A8.3)
V25. `Command::parse_from_markdown` roundtrips 3 user fixture commands byte-stable. (A8.4)
V26. Content-schema sniffers detect format from bytes for xlsx/csv/pdf/docx/pptx/jsonl in fixture corpus. (A8.5)
V27. End-to-end eval: `tdw-eval-runner --dataset fixtures/eval/simple_math.parquet --agent fixtures/agent/calc.json` writes a complete `EvalRun` to CH with all metrics populated. (A8.6)
V28. Workflow cycle-detection rejects a 3-node cyclic graph with a precise error pointing at the offending edge. (A8.7)
V29. Storage mapping audit: every type listed in §4.2 has a corresponding migration + CRUD path; CI verifies via `cargo test -p tdw-agent-store --test storage_mapping`. (A8.8)
V30. Gotcha registry: at least 10 entries seeded; nightly reactor job marks 0 as Stale on first run; CI verifies the auto-Stale path with a fake "old" entry. (A8.9, R21)
V31. MCP smoke: `tdw-mcp tools/list` lists all 10 `tdw.agents.*` tools; `tools/call tdw.agents.search_skills` returns ranked results. (A8.10)
V32. JSON Schema published to `docs/agent-schemas/` matches Rust definitions byte-stable. (A8.1)

---

## 9. Open Questions

- **O6**: Authoring UX — should the user be able to edit Skills/Commands/Prompts via a CLI/TUI (`tdw-cli agent edit skill foo`) or only via direct file edit? Default assumption: file edit + git, since that matches Claude Code's pattern.
- **O7**: Eval runner — local-only at Phase 8, or also support routing through `tdw-embed-openai`/`tdw-embed-google`-style API agents from day one? Default: local-only at Phase 8, hosted-LLM agents land at Phase 9.
- **O8**: dbt vs SQLMesh — should we evaluate SQLMesh as an alternative? SQLMesh has stronger lineage, semver-versioned models, and time-travel; dbt has the bigger ecosystem. Default: dbt for Phase 7, SQLMesh re-evaluation note in ADR-0011 follow-ups.
- **O9**: Gotcha registry seed — pull initial 10 entries from the user's memory / prior sessions, or have the user supply? Default: seed from memory (which already has system-reminder hooks about common issues), user reviews + approves.
- **O10**: Agent Card adoption — A2A is one of several emerging standards (OpenAI's tool card, Anthropic's MCP server card). Should we support multiple, or pick one and document the conversion? Default: A2A as canonical; converters in `tdw-agent` for the other two as derives.

---

## 10. Combined timeline (parent v2.1 + this extension)

| Phase | Description | Days | Cumulative |
|-------|-------------|------|------------|
| 0.0   | Discovery (BOM re-derive, ADRs, license) | 0–2 | 2 |
| 0.1   | Workspace skeleton + CI matrix | 1 | 3 |
| 1     | Core abstractions (Fetcher, Streamer, traits, runtime) | 2–5 | 8 |
| 2     | Storage engines (CH + PG specialist split) | 6–10 | 13 |
| 3     | First providers (fileset + Polygon/Yahoo + WS mock) | 11–13 | 16 |
| 4     | Hybrid retrieval (Qdrant + Meili + S3 + 3 embedders) | 14–20 | 23 |
| 5     | Four consumer shells (service + worker + MCP + CLI) | 21–26 | 29 |
| 6     | Hardening & docs | 27–32 | 35 |
| **7** | **Data engineering (dbt, SQL, ETL/ELT, DDL codegen)** | **33–42** | **45** |
| **8** | **Agent schemas (12 types, MCP tools, eval runner)** | **43–49** | **52** |

Total ~52 days at the indicated cadence. Phases 7 and 8 can run in parallel after Phase 6 if you split work — phase 7 is dbt/SQL-heavy (one workstream), phase 8 is Rust-heavy (a different workstream). With parallelization, total → ~42 days.

---

## 11. References

**Parent plan**
- `C:\Users\ReyDa\FinX-Finance\.plans\2026-05-21-rust-trading-data-warehouse.md` (v2.1)

**Data engineering**
- dbt docs: https://docs.getdbt.com/
- dbt-clickhouse: https://github.com/ClickHouse/dbt-clickhouse
- dbt-postgres: https://github.com/dbt-labs/dbt-postgres
- dbt-utils, dbt-expectations: https://hub.getdbt.com/
- Medallion architecture: Databricks lakehouse paper; same pattern works on PG/CH.
- SQLMesh (alternative for re-eval): https://sqlmesh.com/

**Agent schemas — protocol references**
- A2A (Agent2Agent) Protocol — `agent.json` format: https://google.github.io/A2A/
- Model Context Protocol (MCP) — `tools/`, `resources/`, `prompts/`: https://modelcontextprotocol.io/
- Anthropic tool use spec — function calling format
- OpenAI function calling — JSON Schema-based tool spec
- Google Gemini function calling — similar JSON Schema-based

**Rust crates anchoring this extension**
- `sqlx` (Postgres migrations + queries)
- `clickhouse` (Rust client)
- `refinery` or custom CH migration tool
- `serde_yaml` + `pulldown-cmark` (SKILL.md parsing)
- `minijinja` (prompt templating)
- `petgraph` (workflow DAG validation)
- `schemars` (JSON Schema export)
- `validator` (validation derive)
- `ulid` (run/item IDs)

---

## 12. ADR — Architecture Decision Record (extension)

- **Decision**: Add a data-engineering layer (dbt + SQL + ETL/ELT + DDL codegen) as Phase 7, and an agent-schema layer (12 Rust types + storage + MCP tools + eval runner + workflow engine) as Phase 8, on top of the v2.1 FinX-Finance core.
- **Drivers**:
  1. Raw provider data alone is not query-useful; analysts and downstream agents need conformed, joined, tested marts.
  2. Agents are first-class domain entities in this project (you have many skills, commands, evals); modeling them as data avoids a parallel filesystem-only world.
  3. Eval observability is a foundational capability — without it, you cannot trust agent quality changes.
- **Alternatives considered**:
  - **No data-engineering layer; rely on direct ClickHouse views**: rejected — analytics drift, no testing, no lineage.
  - **dbt-only, no Rust DDL codegen**: rejected — schema becomes split-brain (Rust structs + dbt sources drift).
  - **SQLMesh instead of dbt**: deferred — dbt ecosystem is larger; re-evaluate post-Phase 7 (ADR-0011 follow-up).
  - **Agent schemas as 12 separate crates**: rejected — over-fragmented; one crate with 12 modules has the same compile-time guarantees with less Cargo.toml ceremony.
  - **Agent metadata in JSON files only (no DB)**: rejected — defeats search, eval traceability, and gotcha registry observability.
- **Why chosen**:
  - The medallion pattern is the industry default and has the best dbt support.
  - Single `tdw-agent` crate keeps things navigable without sacrificing modularity.
  - Postgres for metadata + ClickHouse for runs is the established pattern (also matches what the parent plan already builds).
  - MCP tools auto-derive from agent schemas → zero new glue for LLM consumption.
- **Consequences**:
  - +7 crates in workspace (29 total).
  - +17 days (Phases 7 + 8); parallelizable to +10 days.
  - DDL becomes Rust-canonical; dbt sources reference the generated tables (V21 gate enforces).
  - Eval observability is in ClickHouse, not Postgres → fits with the storage matrix (R20 mitigates blow-up).
- **Follow-ups**:
  - ADR-0011 dbt dispatch rule (pg vs ch per model).
  - ADR-0012 one-crate-vs-many for `tdw-agent`.
  - ADR-0013 storage split for evals (PG metadata + CH runs + S3 large items).
  - ADR-0014 Agent Card protocol choice (A2A canonical + converters).
  - Open questions O6–O10 (above).

---

## 13. Changelog

**2026-05-21 — initial direct-mode plan (Layer A + Layer B extension)**
- 11 agent types as a single `tdw-agent` crate with 12 modules.
- Data engineering layer with dbt + medallion architecture (bronze/silver/gold) on Postgres + ClickHouse.
- DDL codegen from `tdw-domain` Rust structs (single source of truth).
- ETL/ELT orchestration via RiverQueue + dbt CLI runner with typed `run_results.json` parsing.
- MCP tool family `tdw.agents.*` exposes agent metadata + skill search + eval leaderboard.
- 11 new acceptance criteria for Layer A (A7.1–A7.10), 11 for Layer B (A8.1–A8.11).
- 11 new risks (R15–R25); 18 new verification steps (V15–V32).
- ADR section + 5 open questions (O6–O10).
