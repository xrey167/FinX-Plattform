# FinX-Finance — Knowledge Graph + Dynamic Feature Tagging (Layer G)

**Project:** FinX-Finance (`C:\Users\ReyDa\FinX-Finance\`)
**Date:** 2026-05-21
**Mode:** `/omc-plan --direct` (new Phase 16 + cross-cutting amendments)
**Status:** Draft — depends on Phase 9 (spine), Phase 12 (graph), Phase 4 (hybrid search)
**Parent plans:**
- [`2026-05-21-rust-trading-data-warehouse.md`](./2026-05-21-rust-trading-data-warehouse.md) — core
- [`2026-05-21-data-engineering-and-agent-schemas.md`](./2026-05-21-data-engineering-and-agent-schemas.md) — Layer A+B
- [`2026-05-21-hook-event-spine.md`](./2026-05-21-hook-event-spine.md) — Layer E (Phase 9, substrate for rules)
- [`2026-05-21-databend-surrealdb-feature-parity.md`](./2026-05-21-databend-surrealdb-feature-parity.md) — Layer C (Phase 12 graph is foundation)
- [`2026-05-21-test-strategy.md`](./2026-05-21-test-strategy.md) — Layer F (binds this phase to coverage gates)

---

## 1. Goal

Build a **unified knowledge graph + dynamic feature-tagging layer** that:

1. **Catalogs every domain entity** (Instrument, Company, Sector, Person, Event, Document, Strategy, Position, Trade, etc.) and the typed relationships between them — a single semantic layer over Postgres + ClickHouse + Qdrant + S3.
2. **Tags every record and every event** with a customizable taxonomy of feature labels — static (manual), computed (rule-derived), AI-generated (LLM classification), and feature-store (ML features). Tags are first-class queryable attributes.
3. **Reacts dynamically** — tag rules are declarative, hot-reloadable at runtime via the event spine (Phase 9), no recompile needed.
4. **Integrates everywhere**:
   - Hybrid search (Phase 4) filters and re-ranks by tags.
   - Spine events (Phase 9) carry tag mutations and tag-derived signals.
   - dbt models (Phase 7) join tags as dimensions for analytics marts.
   - Agent skills (Phase 8) declare tag interests + emit tags.
   - Graph traversal (Phase 12) treats tags as a parallel relationship type.
   - Live queries (Phase 11) subscribe to tag changes.
5. **Customizable + extendable** — users define their own tag taxonomies and rules without touching the warehouse code. Schema-first (taxonomy in Postgres) + behaviour-first (rules as data, hot-reloadable) duality.

This layer is **foundational for agentic workflows**: agents reason in terms of tagged entities, not raw rows. It is **load-bearing for retrieval quality**: hybrid search becomes structured + lexical + semantic + symbolic (tags) with deterministic re-ranking. It is **the answer to "how does the warehouse acquire ML-shaped features"**.

---

## 2. RALPLAN-DR Summary

### Principles
1. **One catalog, many surfaces.** The knowledge graph is a logical layer over physical storage — entities live where they always lived (PG for reference, CH for events, S3 for blobs), the KG is the typed view + index that ties them.
2. **Tags are data, not code.** Tag taxonomies and rules are rows in Postgres; runtime hot-reload via spine; users add/edit without touching Rust.
3. **Lattice, not tree.** Tags form a directed acyclic graph (DAG) with multi-inheritance — a tag can have multiple parents (e.g., `event/earnings` is both `event/*` and `finance/quarterly`). No forced single hierarchy.
4. **Provenance is mandatory.** Every tag assignment records *who* (Actor from Layer E), *what rule* (Definition ID), *when*, and *confidence*. No anonymous tags.
5. **Specialists for execution, generalist for catalog.** KG catalog + tag store live in Postgres for transactional integrity. Tag-derived signals stream through the spine. Entity embeddings + tag embeddings live in Qdrant for semantic search. Tag analytics aggregate in ClickHouse.

### Decision Drivers (top 3)
1. Agents need a typed, queryable semantic layer over raw rows — without it, every agent reimplements entity resolution and filtering.
2. Retrieval quality (Phase 4) plateaus without symbolic re-ranking — tags are the lever.
3. ML feature engineering (eval runner, Phase 8) needs a feature store; tags are the storage primitive.

### Viable Options

#### Option A — Unified KG + tag store layered over the existing stack *(chosen)*
- **Pros**: builds on already-planned `tdw-graph` (Phase 12); reuses spine for rule execution; reuses Qdrant + Meili for tag search; declarative; hot-reloadable.
- **Cons**: cross-cutting — touches 6+ existing crates; net ~12 days of work; adds 4–5 new crates.

#### Option B — Standalone Neo4j sidecar for KG; tags via JSONB columns only
- **Pros**: mature graph DB; standard Cypher.
- **Cons**: introduces a service to run; violates §3 of core plan ("specialist primitives over multi-model generalists"); BSL-style licensing concerns; cross-process latency on every traversal; nothing to gain over Phase 12's typed edges + Postgres.
- **Invalidation rationale**: clean-room scope rejects standalone-DB additions when the existing stack covers the need.

#### Option C — Skip KG; tags only (lightweight)
- **Pros**: cheaper, faster to ship.
- **Cons**: agents still need entity resolution (e.g. "AAPL" and "Apple Inc." are the same); tagging without canonical entities produces duplicate work; retrieval re-ranking by tags has no fixed pivot.
- **Invalidation rationale**: tagging without an entity catalog is shallower than the user asked for ("fully integrated knowledge graph system"); both are needed.

---

## 3. Architecture

```
                ┌──────────────────────────────────────────────────────────┐
                │              CONSUMERS                                   │
                │  axum / tonic · MCP · CLI · dbt · agents · live queries  │
                └────────────────┬─────────────────────────────────────────┘
                                 │ query: "entities tagged X near Y"
                                 ▼
   ┌───────────────────────────────────────────────────────────────────────┐
   │                  KG QUERY LAYER (tdw-kg)                              │
   │  - Cypher-lite Rust API: g.find::<Instrument>().tagged("event/...").  │
   │    related_to::<Company>("issued_by").within(...)                     │
   │  - Cost-based plan over PG + CH + Qdrant + Meilisearch                │
   │  - Tag-aware re-ranker for hybrid search                              │
   └────────────┬─────────────────────────────────┬────────────────────────┘
                │                                 │
                ▼                                 ▼
   ┌──────────────────────────┐   ┌────────────────────────────────────────┐
   │ KG CATALOG (Postgres)    │   │  TAG STORE (Postgres + mirrors)        │
   │ - entity_type            │   │  - tag_definition (taxonomy DAG)       │
   │ - entity                 │   │  - tag_assignment (entity ↔ tag)       │
   │ - relationship_type      │   │  - tag_rule (declarative)              │
   │ - relationship           │   │  - tag_provenance (who/what/when)      │
   │   (edges from Phase 12)  │   │  + Qdrant mirror: tag embeddings       │
   │ - canonical_alias        │   │  + Meili mirror: tag lexical search    │
   └──────────────────────────┘   │  + CH mirror: tag analytics            │
                                  └────────────────┬───────────────────────┘
                                                   │
                                                   ▼
                       ┌──────────────────────────────────────────────────┐
                       │   TAG RULES ENGINE (tdw-tag-rules)               │
                       │   - DSL (TOML/YAML) → compiles to spine hooks    │
                       │   - hot reload via system.tag_rule + spine event │
                       │   - rule kinds: on_event, on_schedule, on_demand │
                       │   - LLM-backed rule type for AI tagging          │
                       └────────────────┬─────────────────────────────────┘
                                        │
                                        ▼
                          ┌────────────────────────────────┐
                          │     EVENT SPINE (Phase 9)      │
                          │  - tag.applied  · tag.removed  │
                          │  - tag.rule_added · tag_eval   │
                          │  - entity.resolved             │
                          └────────────────────────────────┘
```

### Mental model

- **Entity catalog** = the typed nouns (Instrument, Company, …). Storage is PG `entity` table with a `kind` discriminator and an `attributes JSONB`; specific fields per kind are GENERATED columns. Each entity has a stable `id` (ULID) + zero-or-more `canonical_alias` rows (for entity resolution: AAPL → Apple Inc.).
- **Relationship catalog** = typed edges (Phase 12's `tdw-graph` provides RELATE/->/<->); KG layer adds a higher-level Rust API + cost-based planner.
- **Tag store** = `tag_definition` (taxonomy nodes, DAG via `parent_ids[]`) + `tag_assignment` (entity ↔ tag, with `source`, `confidence`, `applied_by`, `applied_at`, optional `expires_at`).
- **Tag rules** = declarative rows in `tag_rule` table. Each rule has `trigger` (event kind or cron), `predicate` (jsonpath or SQL), `action` (apply/remove tag), and `enabled` flag. Runtime hot-reload via `system.tag_rule` change events (spine eats own dogfood, again).
- **AI tags** = a special rule kind whose action is "invoke this LLM prompt template, parse output, apply tags from a controlled vocabulary". Sandboxed by `tdw-udf-external` (Phase 14).
- **Feature store** = a thin view over `tag_assignment` filtered to `kind = 'feature'`; each ML feature is a tag with a numeric `value` field. Used by `tdw-eval-runner` (Phase 8).

---

## 4. Schemas (concrete)

```rust
// crates/tdw-kg/src/entity.rs

#[derive(Serialize, Deserialize, JsonSchema, Validate, sqlx::FromRow)]
pub struct Entity {
    pub id: Ulid,
    pub kind: EntityKind,            // Instrument | Company | Sector | Person | Event | Document | Strategy | Position | Trade | ...
    pub canonical_name: String,
    pub attributes: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub source_records: Vec<SourceRef>,  // pointers to PG/CH/S3 rows backing this entity
}

#[derive(Serialize, Deserialize, JsonSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Instrument,    // tradable security
    Company,
    Sector,
    Person,        // analyst, executive, central banker
    Event,         // earnings, FOMC, M&A
    Document,      // filing, transcript, research note
    Strategy,      // user-defined trading strategy
    Position,
    Trade,
    Topic,         // semantic clusters
    Custom(String),// user-defined kinds at runtime
}

#[derive(Serialize, Deserialize, JsonSchema, sqlx::FromRow)]
pub struct CanonicalAlias {
    pub entity_id: Ulid,
    pub alias: String,           // "AAPL", "Apple", "Apple Inc.", "AAPL US Equity"
    pub kind: AliasKind,         // Ticker | LegalName | CommonName | Cusip | Isin | Lei | UserDefined
    pub source: String,
    pub confidence: f32,
}
```

```rust
// crates/tdw-tags/src/tag.rs

#[derive(Serialize, Deserialize, JsonSchema, Validate, sqlx::FromRow)]
pub struct TagDefinition {
    pub id: Ulid,
    pub key: String,                          // "event/earnings", "sentiment/positive", "feature/momentum_5d"
    pub display_name: String,
    pub description: String,
    pub kind: TagKind,                        // Static | Computed | Ai | Feature | Lifecycle
    pub parent_ids: Vec<Ulid>,                // DAG — multi-inheritance
    pub value_schema: Option<serde_json::Value>, // JSON Schema if tag carries a value
    pub taxonomy: String,                     // "events" | "sentiment" | "factors" | user-defined
    pub default_ttl: Option<Duration>,        // tags can expire
    pub created_by: Actor,
    pub created_at: DateTime<Utc>,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, sqlx::Type)]
#[serde(rename_all = "snake_case")]
pub enum TagKind {
    Static,    // applied manually by users
    Computed,  // applied by deterministic rules
    Ai,        // applied by LLM classifier
    Feature,   // ML feature (carries numeric value)
    Lifecycle, // applied by system (e.g., "ingested", "validated", "indexed")
}

#[derive(Serialize, Deserialize, JsonSchema, Validate, sqlx::FromRow)]
pub struct TagAssignment {
    pub id: Ulid,
    pub entity_id: Ulid,
    pub entity_kind: EntityKind,              // denormalized for fast filtering
    pub tag_id: Ulid,
    pub tag_key: String,                      // denormalized
    pub value: Option<serde_json::Value>,     // populated if tag has value_schema
    pub confidence: f32,                      // 0.0–1.0
    pub source: TagSource,                    // who/what assigned
    pub applied_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub correlation_id: Uuid,                 // ties to the event chain that produced it
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TagSource {
    Manual { actor: Actor },
    Rule { rule_id: Ulid, rule_version: SemVer },
    LlmClassifier { prompt_id: Ulid, model: String },
    External { system: String, request_id: String },
}
```

```rust
// crates/tdw-tag-rules/src/rule.rs

#[derive(Serialize, Deserialize, JsonSchema, Validate, sqlx::FromRow)]
pub struct TagRule {
    pub id: Ulid,
    pub name: String,
    pub version: SemVer,
    pub kind: RuleKind,
    pub trigger: RuleTrigger,
    pub predicate: Predicate,
    pub action: RuleAction,
    pub enabled: bool,
    pub priority: i32,             // lower = earlier (consistent with Phase 9 hooks)
    pub max_depth: u8,
    pub author: Actor,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleTrigger {
    /// Fires when a spine event of the given kind is emitted.
    OnEvent { event_kind: String },
    /// Fires on cron schedule (re-evaluates all matching entities).
    OnSchedule { cron: String },
    /// Fires manually via CLI or API.
    OnDemand,
    /// Fires when another tag is added or removed.
    OnTag { tag_key: String, change: TagChange },
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Predicate {
    /// Pure SQL predicate evaluated against the entity row.
    Sql { expr: String },
    /// JsonPath against the event payload or entity attributes.
    JsonPath { path: String, op: JsonOp, value: serde_json::Value },
    /// Compound: all/any/none of children.
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    None_(Vec<Predicate>),
    /// LLM classifier: feeds the entity context to a prompt, parses output.
    LlmClassifier { prompt_id: Ulid, threshold: f32 },
}

#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleAction {
    Apply { tag_key: String, value: Option<serde_json::Value>, ttl: Option<Duration> },
    Remove { tag_key: String },
    Compose(Vec<RuleAction>),
}
```

### Postgres schema (DDL summary)

```
agents.entity                   -- KG catalog
agents.canonical_alias          -- entity resolution
agents.entity_relationship      -- typed edges (from Phase 12)
agents.tag_definition           -- taxonomy DAG
agents.tag_assignment           -- entity ↔ tag with provenance + ttl
agents.tag_rule                 -- declarative rules
system.tag_rule_state           -- runtime enable/disable + version
events.tag_change               -- ClickHouse mirror for analytics
```

Indexes:
- `tag_assignment(entity_id)` btree
- `tag_assignment(tag_key) WHERE expires_at IS NULL OR expires_at > now()` partial — live tags only
- `tag_assignment USING GIN (value jsonb_path_ops)` — when tag has a value
- `entity USING GIN (attributes jsonb_path_ops)`
- `canonical_alias(LOWER(alias))` — case-insensitive lookup
- `entity_relationship(from_id, to_id, kind)` btree

---

## 5. Rules Engine — how it actually runs

The Tag Rules Engine compiles **rows** in `agents.tag_rule` into **spine hook registrations** (Phase 9). At startup:

1. `tdw-tag-rules::loader` reads all enabled rules from `agents.tag_rule`.
2. For each rule, dispatches by `trigger`:
   - `OnEvent { event_kind }` → registers a `#[hook(on = <event_kind>, kind = Action, tx = PostCommit, side = Both)]`-equivalent runtime handler. Handler evaluates `predicate` against `EventEnvelope.payload` + entity context; if matched, executes `action`.
   - `OnSchedule { cron }` → registers a RiverQueue periodic job.
   - `OnDemand` → exposes as CLI: `tdw-cli tag rule run <rule_id> --entity <id>`.
   - `OnTag` → registers a hook on `tag.applied` / `tag.removed` spine events.
3. Rule changes (`system.tag_rule_state` updates or `agents.tag_rule` upserts) emit a `tag.rule_changed` spine event; the loader subscribes and **hot-reloads the affected handlers** without restart.

### Compile-time + runtime duality

- Rules **declared statically** in code (Rust function with `#[tag_rule]` attribute) generate `tag_rule` rows on first run + are auto-linked to a compiled handler.
- Rules **declared at runtime** via API (`POST /v1/tag-rules`) are pure data + handler is generated from the rule's JSON shape via `tdw-tag-rules::Engine` (no codegen, dispatched at runtime).

This dual surface keeps performance-critical built-in rules compiled while letting users add rules through a UI / CLI / agent without recompiling.

### LLM classifier rules

A `Predicate::LlmClassifier { prompt_id, threshold }` runs as follows:
1. Loads the prompt template from `agents.prompt` (Phase 8 schema).
2. Renders with entity context as variables.
3. Calls the configured LLM provider (`tdw-embed-openai` / `tdw-embed-google` / local) — actually re-uses `tdw-udf-external` (Phase 14) for safety + retry + rate limiting.
4. Parses output as `{ matched: bool, confidence: f32, reasoning: string }`.
5. If `matched && confidence >= threshold`, predicate passes.
6. Provenance: every LLM tag assignment records `source = LlmClassifier { prompt_id, model }` + the full response (truncated) in `event_archive` (Phase 9).

---

## 6. Integration with existing layers

### Hybrid search (Phase 4) — tag-aware re-ranking

`tdw-storage-router::HybridSearch::search(query, filters: TagFilter, top_k)`:

1. Lexical pass (Meilisearch) returns top-K candidates.
2. Vector pass (Qdrant) returns top-K candidates.
3. Tag filter applies (`tag_key IN (...)`, `tag_key NOT IN (...)`, `tag.value > 0.5`).
4. RRF fusion (already in Phase 4) with one new term: `tag_score = sum(matched_tag.weight * tag.confidence)`.
5. Returns ranked results with score breakdown including `tag_contribution`.

### Spine (Phase 9) — tag mutations as first-class events

New event kinds:
- `tag.applied { entity_id, tag_key, value, source, correlation_id }`
- `tag.removed { entity_id, tag_key, reason, source }`
- `tag.rule_added { rule_id, version }` / `tag.rule_updated` / `tag.rule_removed`
- `tag.eval_complete { rule_id, entities_changed, latency_ms }`
- `entity.resolved { entity_id, aliases_added }`
- `entity.merged { primary_id, merged_id }`

All carry full `Actor` context. Downstream subscribers (e.g., a live dashboard) react via `LIVE SELECT * FROM tag_change WHERE entity_id = ...`.

### Graph (Phase 12) — tags as a parallel relationship

Phase 12 ships typed relationships (`RELATE alice -> knows -> bob`). Tags are *another* relationship form between an entity and a tag-definition node:

```
entity(AAPL:Instrument) ──tagged──> tag_definition(event/earnings)
                                              │
                              parent_of │      │ child_of
                                              ▼
                                     tag_definition(event/*)
```

Recursive traversal `(:Instrument) -[:tagged]-> () -[:child_of*0..]-> ()` returns all entities tagged with anything matching a tag prefix — a graph-native answer to "all earnings events" or "all sentiment-positive instruments".

### dbt (Phase 7) — tags as joinable dimensions

A dbt source declaration on `agents.tag_assignment` + a model `dim_entity_tags` pivots assignments into wide tables for analytics:

```sql
-- models/silver/agents/silver_entity_tags.sql
select
  entity_id,
  entity_kind,
  bool_or(tag_key = 'event/earnings')                                              as has_earnings,
  bool_or(tag_key like 'sentiment/%')                                              as has_sentiment,
  max(case when tag_key = 'sentiment/positive' then confidence else 0 end)         as sentiment_positive_conf,
  array_agg(distinct tag_key)                                                       as tag_keys
from {{ source('agents', 'tag_assignment') }}
where expires_at is null or expires_at > now()
group by 1, 2
```

### Agent schemas (Phase 8) — tag interests + emission

`Skill` (Phase 8) gains optional fields:
```rust
pub struct Skill {
    // ... existing fields ...
    pub tag_interests: Vec<String>,    // skill subscribes to entities tagged with these
    pub tag_emissions: Vec<String>,    // skill may emit these tags
}
```

MCP tools: `tdw.agents.subscribe_to_tagged(tag_keys, callback)` — agent reacts to new entities matching a tag pattern.

### Live queries (Phase 11) — subscribe to tag mutations

`LIVE SELECT * FROM tag_assignment WHERE entity_id = 'aapl' AND tag_key LIKE 'sentiment/%'` — clients see real-time tag updates with permission filtering.

### Eval runner / feature store (Phase 8)

ML features are tags with `kind = Feature` and a numeric `value`. `tdw-feature-store` is a thin Rust crate that:
- Lists features for an entity: `tag_assignment WHERE entity_id = X AND tag_kind = 'Feature'`.
- Subscribes to feature changes via spine.
- Versions features by tag version.
- Provides time-travel queries (current value vs at snapshot T): integrates with Phase 10 snapshots.

### UDFs (Phase 14)

`tdw-udf` gains a `TagCapability` that lets a UDF read tags for the entity it's processing and emit new tag assignments (subject to permission checks). Sandboxed UDFs can compute features without bypassing the spine.

---

## 7. New crates

```diff
crates/
+ tdw-kg/                      ← Entity catalog, canonical aliases, KG query API
+ tdw-entity-resolver/         ← Alias matching + entity merge logic
+ tdw-tags/                    ← Tag definitions, assignments, mirrors to Qdrant/Meili/CH
+ tdw-tag-rules/               ← Rule engine, DSL parser, hot-reload loader
+ tdw-feature-store/           ← Thin layer for ML features (kind = Feature tags)
```

Sub-modules / extensions:
- `tdw-runtime::tag_hooks` — built-in spine hooks for entity / tag lifecycle.
- `tdw-storage-router::tag_filter` — tag predicate translation to engine-specific queries.
- `tdw-mcp::agent_tags` — MCP tools for tag CRUD and rule management.
- `tdw-cli::tag` — CLI subcommands: `tdw-cli tag list/apply/remove/rule create/run`.

Total workspace count: ~47 (Layer E) + 5 (this) − 0 = **~52 crates**.

---

## 8. Phase 16 — Knowledge Graph + Dynamic Tagging — days 109–120

Sits after Phase 15 (Hardening). Depends on Phase 9 (spine), Phase 12 (graph), Phase 4 (hybrid search), Phase 8 (agents), Phase 14 (UDF external for LLM classifier).

### Phase 16.1 — Entity catalog (days 109–112)

16.1.1. `tdw-kg::entity` core types: `Entity`, `EntityKind`, `CanonicalAlias`, `SourceRef`. Derive `Serialize/Deserialize/JsonSchema/Validate`. Postgres migration for `agents.entity` + `agents.canonical_alias`.
16.1.2. Bootstrap entity kinds from the 11 BOM schemas (`Instrument`, `Position`, `Trade`, etc.) — auto-derive from `tdw-domain` Rust structs via macro: `#[derive(KgEntity)]` registers the kind + maps source records.
16.1.3. `tdw-entity-resolver` MVP: fuzzy alias matching (Jaro-Winkler), explicit alias upsert API, manual merge tool. **Not** automatic merging in v0.1 — too risky.
16.1.4. KG query API (Cypher-lite Rust): `kg.find::<Instrument>().named("AAPL").or_alias("Apple Inc.").one()`.

### Phase 16.2 — Tag store (days 113–115)

16.2.1. `tdw-tags::tag_definition` schema + Postgres migration. Seed initial taxonomy (10 root tags) including `event/`, `sentiment/`, `factor/`, `lifecycle/`, `quality/`, `feature/`, `topic/`.
16.2.2. `tdw-tags::tag_assignment` with provenance + TTL + correlation_id from spine envelope.
16.2.3. Mirrors:
   - Qdrant collection `tags__{embed_model}` storing embeddings of `tag_definition.description` for semantic tag lookup ("find tags similar to 'volatility'").
   - Meilisearch index `tags` for lexical filtering / autocomplete.
   - ClickHouse `events.tag_change` table for analytics over tag mutations.
16.2.4. CRUD API + MCP tools: `tdw.tags.create_definition`, `tdw.tags.apply`, `tdw.tags.remove`, `tdw.tags.search`, `tdw.tags.taxonomy_tree`.
16.2.5. Tag DAG validation: prevent cycles in `parent_ids`; refuse upsert if cycle would form.

### Phase 16.3 — Rule engine (days 116–118)

16.3.1. `tdw-tag-rules::rule` core types: `TagRule`, `RuleTrigger`, `Predicate`, `RuleAction`.
16.3.2. Loader: reads rules from PG on startup, registers spine handlers (Phase 9) for `OnEvent` triggers, RiverQueue periodic jobs for `OnSchedule`, CLI dispatch for `OnDemand`.
16.3.3. Predicate evaluator:
   - `Sql { expr }` — prepared statement against the entity row + event payload.
   - `JsonPath` — `jsonpath_lib` crate.
   - `All/Any/None_` — short-circuit composition.
   - `LlmClassifier` — delegates to `tdw-udf-external` (Phase 14) with prompt + threshold.
16.3.4. Hot reload: subscribe to `tag.rule_changed` events; reload affected handlers atomically (drop old, register new) within 500 ms.
16.3.5. Recursive-event protection: rule that emits a tag whose `tag.applied` triggers another rule respects Phase 9's `MAXDEPTH = 8` guard; per-rule `max_depth` override allowed.
16.3.6. CLI: `tdw-cli tag rule create --from-yaml ./rule.yaml`, `tdw-cli tag rule enable/disable <id>`, `tdw-cli tag rule run <id> --entity <id>`.

### Phase 16.4 — Integration glue (days 119–120)

16.4.1. Hybrid search re-ranker: `tdw-storage-router::HybridSearch::search` learns a `TagFilter` parameter; RRF fusion adds `tag_contribution` score term.
16.4.2. dbt sources: `agents.entity`, `agents.tag_assignment`, `agents.tag_definition` declared in `models/_sources.yml`; pivoted in `silver_entity_tags.sql` (Phase 7 model added).
16.4.3. Agent skill schema extension: `tag_interests` + `tag_emissions` fields; backwards-compatible (default empty).
16.4.4. MCP tools: full surface for tag CRUD, rule management, KG query, entity search.
16.4.5. Live query bridge: `LIVE SELECT * FROM tag_assignment WHERE …` works through the existing `tdw-service::live` adapter (Phase 11).
16.4.6. `tdw-feature-store` MVP: `FeatureStore::get(entity_id, feature_key, at: Option<DateTime>)` reads from `tag_assignment` with snapshot-aware time travel (Phase 10).
16.4.7. Documentation: `docs/knowledge-graph.md`, `docs/tagging.md`, `docs/tag-rules-cookbook.md` with 10 worked examples (earnings detection, sentiment classification, momentum feature, etc.).

**Exit criteria**: A16.1–A16.20 satisfied.

---

## 9. Acceptance Criteria

A16.1. `tdw-kg::Entity` round-trips through Postgres for all 10 starter `EntityKind`s; verified by serde golden tests.
A16.2. Bootstrap: every `tdw-domain` struct annotated `#[derive(KgEntity)]` produces a row in `entity` on its first write; verified by integration test that writes a `Trade` and asserts the row exists.
A16.3. `tdw-entity-resolver::find_or_create("AAPL")` returns the same entity as `find_or_create("Apple Inc.")` after a manual `merge`; resolves to the same `entity.id`.
A16.4. KG query API: `kg.find::<Instrument>().tagged("event/earnings").related_to::<Document>("mentioned_in").execute()` returns all instruments with earnings events that are mentioned in any document, verified against a fixture set of 100 entities and 30 documents.
A16.5. `tag_definition` DAG validator rejects a cycle: `A.parent_ids = [B], B.parent_ids = [A]` is refused with a precise error pointing at the offending edge.
A16.6. Static tags applied manually by API survive a restart; verified.
A16.7. Tag TTL: a tag with `expires_at = now() + 1m` is filtered out of queries after 1 minute; verified via `tokio::time::pause` integration test.
A16.8. Provenance: every `tag_assignment` row has a populated `source` (Manual/Rule/Llm/External); CI test refuses migration if any row has `source IS NULL`.
A16.9. Rule engine bootstrap: 5 seeded rules (e.g., "tag instrument as `event/earnings` if SEC EDGAR 10-Q published") execute on cold start; verified by integration test running ingest + assertion.
A16.10. Hot reload: changing `tag_rule.enabled` from `true` to `false` stops the rule from firing within 500 ms; verified by emitting trigger events before and after the toggle.
A16.11. Adding a new rule via `POST /v1/tag-rules` activates it within 500 ms without restart.
A16.12. `Predicate::Sql` rule correctly tags 100 entities based on a numeric attribute; verified.
A16.13. `Predicate::JsonPath` rule correctly tags entities based on event payload contents.
A16.14. `Predicate::LlmClassifier` rule with a fixture prompt + mocked LLM returns the right tags for 10 input documents at threshold 0.8; mocked via `tdw-udf-external` test harness.
A16.15. Rule recursion: rule A applies tag X, which triggers rule B that emits tag Y, which would trigger rule A again — `MAXDEPTH = 8` kicks in at hop 9 with `DepthExceeded`; verified.
A16.16. Hybrid search with `TagFilter::Include(["event/earnings"])` filters out non-matching documents from the top-K; verified against a golden retrieval set.
A16.17. dbt `silver_entity_tags` model builds against testcontainers PG with seeded `tag_assignment` rows; row counts match expected pivot.
A16.18. Agent skill with `tag_interests: ["sentiment/positive"]` receives a notification (via MCP) within 2 s of a new tag matching that key being applied; verified.
A16.19. `tdw-feature-store::get(entity_id, "momentum_5d", at: snapshot_id)` returns the feature value as of that snapshot — integrates with Phase 10 time travel; verified against a 10-day backfill.
A16.20. Adversarial: a rule with a `Predicate::Sql` containing `SELECT * FROM pg_user; DROP TABLE foo;` is rejected at insert time by SQL-injection guard; verified.

---

## 10. Risks & Mitigations

| #    | Risk | Likelihood | Impact | Mitigation |
|------|------|-----------|--------|------------|
| R69  | Tag taxonomy grows unbounded → catalog becomes noisy | High | Medium | TTL on tags; periodic compaction job purges expired; CLI `tdw-cli tag taxonomy stats` shows tag-count distribution; soft-cap at 1000 active taxonomy nodes with warning. |
| R70  | LLM classifier rule racks up cost in production | High | High | Per-rule cost budget (`max_usd_per_day`); rule auto-suspends when exceeded; CI uses local mock embedder by default. |
| R71  | Hot-reload races: rule update mid-evaluation produces inconsistent tags | Medium | Medium | Rules versioned (`version: SemVer`); evaluator captures rule version at start; restart-safe upserts; CAS update on `rule_state.version`. |
| R72  | Entity-resolver false positives merge distinct companies (Apple Inc. ↔ Apple Bank) | Medium | High | No automatic merge in v0.1; manual merge tool + audit trail; confidence threshold for alias matching; require user confirm for merges above a threshold. |
| R73  | Cyclic tag DAG slips past validator | Low | Medium | Validator runs on insert + nightly full-graph traversal check; abort migration on cycle. |
| R74  | Rules firing on tag changes form infinite cycle | Medium | High | Phase 9 `MAXDEPTH = 8` already protects; per-rule `max_depth` override; offending events go to DLQ; rule auto-disables after N depth breaches. |
| R75  | Postgres `tag_assignment` table grows large (millions of rows) | High | Medium | Partition by month; CH mirror for analytics queries; archival to S3 after 1 year; TTL purge nightly. |
| R76  | Tag rule SQL predicates have SQL-injection / privilege-escalation risk | Medium | High | Predicates run as `dbt_runner` role with read-only grants; SQL parsed + AST-validated before execution; banned keywords (`DROP`, `ALTER`, `GRANT`, `;`); A16.20 covers. |
| R77  | LLM-tag assignments drift in quality as models change | High | Medium | Tag assignments record `model: String`; periodic re-evaluation job; quality eval dataset per LLM rule; eval-runner (Phase 8) tracks tag-precision over time. |
| R78  | Hybrid search re-ranking with tags makes results non-deterministic across runs | Medium | Medium | Tag weights versioned; query-time tag snapshot ID captured for reproducibility; CI golden tests use a frozen tag snapshot. |
| R79  | Tag changes cause spine event storms | Medium | Medium | Rule-emitted tag events are batched (`tag.applied[]` envelope with N items per commit); per-rule rate limit; alarm on > 10k tag events/sec. |
| R80  | Feature store reads at historical snapshots require coordinated snapshot trees across PG + CH + Qdrant | Medium | High | Snapshot ID is the global key (Phase 10); `tdw-feature-store::get(at: snapshot_id)` resolves to consistent reads across stores via the routing layer; A16.19 covers. |

---

## 11. Verification Steps

V75. `cargo test -p tdw-kg --features integration` — entity catalog + KG query API. (A16.1–A16.4)
V76. `cargo test -p tdw-entity-resolver` — alias matching + merge round-trip. (A16.3)
V77. `cargo test -p tdw-tags --features integration` — taxonomy DAG, assignment CRUD, TTL, mirrors. (A16.5–A16.8)
V78. `cargo test -p tdw-tag-rules --features integration` — bootstrap, hot reload, predicate variants. (A16.9–A16.14)
V79. `proptest` recursion: random rule chains assert `MAXDEPTH` triggers correctly. (A16.15)
V80. `cargo test -p tdw-storage-router --features tags` — tag-aware hybrid search. (A16.16)
V81. dbt integration: `dbt build --select silver_entity_tags` runs in CI against testcontainers PG. (A16.17)
V82. MCP integration: agent subscribes to tag-emission events; receives within 2s. (A16.18)
V83. `cargo test -p tdw-feature-store --features integration` — snapshot-aware feature reads. (A16.19)
V84. **Adversarial SQL injection suite**: 20 hostile rule predicates (DROP, ALTER, etc.) all rejected at insert. (A16.20, R76)
V85. **LLM cost cap**: a rule deliberately configured at `max_usd_per_day = 0.01` auto-suspends after one expensive call. (R70)
V86. **Entity merge audit**: manual merge of two entities preserves both lineages in `canonical_alias`; revert via `tdw-cli kg entity unmerge` works. (R72)
V87. Coverage gate from Layer F §5: `tdw-kg`, `tdw-tags`, `tdw-tag-rules` ≥ 85% line; `tdw-entity-resolver` ≥ 80%. (Layer F)

---

## 12. ADR

- **Decision**: Build a unified knowledge graph (entity catalog + canonical aliases + typed relationships) plus a dynamic tag store (taxonomy DAG + assignments with provenance + rule engine compiled to spine hooks) layered over the existing FinX-Finance stack. Tag rules are rows in Postgres, hot-reloadable via spine events. LLM classifier rules run through Phase 14's `tdw-udf-external` for sandbox + rate limit. Tags integrate into Phase 4 hybrid search re-ranking, Phase 7 dbt marts, Phase 8 agent skills, Phase 9 spine event kinds, Phase 11 live queries, and Phase 12 graph traversal as a parallel relationship type.

- **Drivers**:
  1. Agents need a typed semantic layer over raw rows.
  2. Retrieval quality (Phase 4) requires symbolic re-ranking beyond pure lexical + vector.
  3. ML feature engineering needs a feature store primitive.

- **Alternatives considered**:
  - **B — Neo4j sidecar + JSONB tags**: rejected — adds a service; violates specialist principle.
  - **C — Tags only, no KG**: rejected — duplicate entity work; no canonical pivot for re-ranking.

- **Why chosen**:
  - Reuses Phase 12 graph as substrate; reuses spine for rule execution; reuses Qdrant + Meili + CH for tag mirrors.
  - Schema-first + behavior-first duality: taxonomy is data; rules are data; both hot-reloadable.
  - LLM classifier delegated to `tdw-udf-external` (Phase 14) inherits its safety net.
  - Snapshot-aware feature store (via Phase 10) gives reproducible ML pipelines for free.

- **Consequences**:
  - +5 crates (`tdw-kg`, `tdw-entity-resolver`, `tdw-tags`, `tdw-tag-rules`, `tdw-feature-store`).
  - +12 days (Phase 16); brings total to ~123 days serial / ~85 parallelized.
  - `tag_assignment` row volume can be large — partitioning + TTL strategy mandatory.
  - LLM rule cost is a real operational concern (R70).
  - Entity resolution is manual-merge-only in v0.1; automatic merge is a v0.2 goal.

- **Follow-ups**:
  - ADR-0030 — taxonomy seed (which 10 root tags ship at v0.1?)
  - ADR-0031 — rule-engine DSL format (YAML vs TOML vs custom)
  - ADR-0032 — LLM classifier per-rule cost cap policy
  - ADR-0033 — entity-merge confidence thresholds + manual review workflow
  - O30 — should the KG support multi-tenancy (per-user entity catalogs)? (Default: single-tenant per Layer C non-goals.)
  - O31 — Cypher front-end on top of `tdw-kg` query API? (Default: no for v0.1; the Rust API is enough.)
  - O32 — feature backfill from historical events (replay tag rules over `event_archive` for time-windows that predated the rule)?

---

## 13. Combined timeline (updated)

| Phase | Description | Days | Cumulative |
|-------|-------------|------|------------|
| 0.0–0.1 | Discovery + skeleton | 0–3 | 3 |
| 0.5 | Test foundation | 4–7 | 7 |
| 1–6 | Core / storage / providers / retrieval / shells / hardening | 8–39 | 39 |
| 7 | Data engineering | 40–49 | 49 |
| 8 | Agent schemas | 50–56 | 56 |
| 9 | Hook & event spine | 57–66 | 66 |
| 10 | Snapshots / time travel | 67–75 | 75 |
| 11 | Streams + live (adapters) | 76–79 | 79 |
| 12 | Graph + spatial | 80–87 | 87 |
| 13 | Stages + table formats | 88–97 | 97 |
| 14 | UDFs + auth + DEFINE + masking | 98–107 | 107 |
| 15 | Hardening + E2E flows | 108–112 | 112 |
| **16 (NEW)** | **Knowledge Graph + Dynamic Tagging** | **113–124** | **124** |

Total **~124 days serial / ~85 days parallelized** (Phase 16 can partially overlap with Phase 15's late-stage hardening, since the new crates' early work is independent; Phase 8 agent-skill extension and Phase 7 dbt source addition can land during Phase 16).

---

## 14. Open Questions

- **O30** — Multi-tenancy?  Default: no.
- **O31** — Cypher front-end? Default: no, Rust API suffices.
- **O32** — Tag-rule backfill over `event_archive`? Default: ship a `tdw-cli tag rule backfill --rule <id> --since <ts>` CLI in Phase 16.4 as a one-line addition.
- **O33** — Should `tdw-feature-store` materialize features into ClickHouse for fast bulk reads (vs always querying tag_assignment)? Probably yes for v0.2.
- **O34** — Per-tag ACL (some tags visible only to certain actors)? Default: not in v0.1; piggy-back on `tdw-mask` (Phase 14) post-hoc.

---

## 15. Changelog

**2026-05-21 — Layer G: Knowledge Graph + Dynamic Tagging**
- 5 new crates: `tdw-kg`, `tdw-entity-resolver`, `tdw-tags`, `tdw-tag-rules`, `tdw-feature-store`.
- Unified entity catalog with canonical-alias resolution + typed relationships (reuses Phase 12 graph).
- Tag taxonomy as DAG (multi-inheritance), 5 tag kinds (Static / Computed / Ai / Feature / Lifecycle).
- Rule engine compiles declarative rules to Phase 9 spine hooks; hot-reloadable via spine.
- LLM classifier rule type delegates to Phase 14 `tdw-udf-external` for sandbox + cost control.
- Tag-aware re-ranking integrated into Phase 4 hybrid search.
- dbt source declarations (Phase 7); agent skill extensions (Phase 8); live query support (Phase 11); spine event kinds (Phase 9).
- 20 acceptance criteria (A16.1–A16.20), 12 risks (R69–R80), 13 verification steps (V75–V87), 4 follow-up ADRs (0030–0033) + 5 open questions (O30–O34).
- Total project timeline: ~111 → **~124 days serial / ~85 days parallelized**.
