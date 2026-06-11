# Knowledge Graph And Tags

The knowledge-system (stories A1–F1) ships a unified graph-backed KG and tag
layer. All entity, relationship, and tag state lives in the graph engine
(Bolt/Memgraph in production; `InMemoryGraphEngine` in dev/test). There is no
longer a Postgres KG table; the legacy tables
(`kg_entity`, `kg_relationship`, `kg_merge_audit`, `tag_definition`,
`tag_assignment`, `tag_rule`) are dropped by the operator script
`sql/ops/drop-kg-pg-tables.sql` after the F1 cutover. The `feature_snapshot`
table (feature-store evidence) is kept.

## Crate responsibilities

| Crate | Responsibility |
|---|---|
| `tdw-kg` | Entity catalog over the 51-kind taxonomy, relationships, neighbor queries, audited manual merges (alias union, edge rewiring, tombstones). Merges remain explicitly approved; nothing auto-merges. |
| `tdw-entity-resolver` | Deterministic resolver candidates and explicit manual merge decisions. |
| `tdw-tags` | Tag definitions, parent DAG validation, assignments, TTL checks, provenance, and taxonomy stats. |
| `tdw-tag-rules` | Hot-reloadable rules with SQL-injection guard and deterministic label/JSON/SQL-style predicates; loaded from the file system at startup (in-memory only, no Postgres backing). |
| `tdw-feature-store` | Feature snapshots enriched with active tags, backed by `feature_snapshot` in Postgres. |
| `tdw-storage-graph` | `GraphEngine` trait + `InMemoryGraphEngine` (always compiled) + `BoltGraphEngine` (behind the `bolt` feature, requires Memgraph/Neo4j). |
| `tdw-infer` | Inference rules and version stamps wired into `KnowledgeVersions`. |
| `tdw-retrieve` | Hybrid `Retriever`: combines vector (Qdrant), lexical (Meilisearch), and graph (BFS neighborhood) results with configurable weights; wired into `KnowledgeRuntime`. |
| `tdw-knowledge` | `KnowledgeRuntime` (hybrid Retriever + graph/tag handles + version triple), `KnowledgeIndexer` (content-hash idempotency, rule-driven auto-tagging, lexical+graph co-index). |

## Graph engine selection

Configured via `[knowledge.graph]` in the daemon TOML (see `tdw-config`):

```toml
[knowledge.graph]
backend = "bolt"                         # or "in-memory"
bolt_uri = "bolt://127.0.0.1:7687"
bolt_user = ""
bolt_password_env = "TDW_GRAPH_PASSWORD"
```

- `bolt` (production default): connects to Memgraph/Neo4j at `bolt_uri`.
  Hard `Init` error if unreachable — no silent fallback.
- `in-memory` (dev/test default): ephemeral `InMemoryGraphEngine`, resets on
  restart.
- Unknown backend value: hard `Init` error at daemon startup.

The `tdw-backend` crate gates the `BoltGraphEngine` arm behind the `bolt`
Cargo feature. Production builds and Dockerfiles enable it; the default
(offline) build compiles only `InMemoryGraphEngine`.

## MCP surface

The knowledge read tools and the `tdw.kg.feedback` write tool (write-back gate
requires the `write_tools_enabled` operator flag, off by default for the agent
surface) are exposed via `McpServer::with_knowledge` /
`McpServer::with_feedback_store`. In the unified `tdw-backend` (`Surfaces::Both`
mode) these handles are injected from the co-resident `Backend` directly — no
loopback round-trip for knowledge operations.

## Retrieval feedback loop

`tdw-agent-store::RetrievalFeedbackStore` records per-query signal (hit rank,
accepted, skipped). The daemon's consolidation scheduler flushes pending
feedback into the knowledge index on its hourly tick (configurable via
`TDW_CONSOLIDATION_TICK_SECS`). The `tdw.kg.feedback` MCP tool gates writes
behind `write_tools_enabled` so agent surfaces default to read-only.

## Postgres migration history

Migration `20260521_0007_kg_tags_feature_store.sql` created the legacy KG/tag
tables. Those tables are historical record only after F1; run
`sql/ops/drop-kg-pg-tables.sql` to remove them from the live schema once all
deployed service versions have been updated to F1.
