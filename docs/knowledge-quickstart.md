# Knowledge System Quickstart

This page gets a new user from zero to a running knowledge graph in four steps:
configure, ingest, search, answer. It is the canonical entry point; the deeper
reference docs (`docs/ops/graph-db.md`, `docs/kg-tags.md`,
`docs/llm-knowledge.md`) cross-link back here.

## 1. Configure

The daemon reads knowledge settings from the `[knowledge]` section of its TOML
config (pointed to by `TDW_CONFIG`, or inlined via `TDW_CONFIG_CONTENT`).

### Zero-config first run (in-memory, no external services)

No config needed. By default the daemon uses an in-memory graph backend and the
deterministic `hash` embedder — both work fully offline. A notice is printed to
stderr at startup to remind you that graph data is not persisted:

```
[tdw] NOTICE: knowledge graph running in-memory — data is NOT persisted
 across restarts. Set [knowledge.graph] backend="bolt" for production.
 See docs/ops/graph-db.md.
```

This is an **explicit default** (knowledge-system K-E1), not a silent fallback.
Both `in-memory` and `bolt` are first-class backends.

### Production config (bolt + embedder)

Create a daemon TOML file and set `TDW_CONFIG=/path/to/config.toml`:

```toml
[knowledge.graph]
backend    = "bolt"
bolt_uri   = "bolt://127.0.0.1:7687"
bolt_user  = ""
# Password is read from the environment, not stored in the file:
bolt_password_env = "TDW_GRAPH_PASSWORD"

[knowledge.embedding]
provider = "hash"          # or: local | openai | google — see Embedder Matrix below
```

Start Memgraph alongside the daemon:

```bash
docker compose --profile full up -d memgraph
```

The `full` and `live` Docker Compose profiles set `backend = "bolt"` and
`bolt_uri = "bolt://memgraph:7687"` automatically via `TDW_CONFIG_CONTENT`.

## 2. Embedder Matrix

| Provider | Build feature | Env var required | When to use |
|---|---|---|---|
| `hash` | always compiled | none | **Default.** Deterministic, offline. No semantic similarity — keywords only. Use for dev/CI or when semantic search is not needed. |
| `local` | `local-model` | `TDW_EMBED_MODEL_DIR` (path to `config.json` + `tokenizer.json` + `model.safetensors`) | On-device semantic embeddings (Candle/BERT). No API key. Requires a model download. |
| `openai` | `openai` | `TDW_OPENAI_EMBEDDING_API_KEY` or `OPENAI_API_KEY` | Cloud semantic embeddings (OpenAI `text-embedding-3-small`). **Fails at startup if the key is unset** — no fallback to hash. |
| `google` | `google` | `TDW_GOOGLE_EMBEDDING_API_KEY`, `GOOGLE_API_KEY`, or `GEMINI_API_KEY` | Cloud semantic embeddings (Gemini `gemini-embedding-001`). **Fails at startup if the key is unset** — no fallback to hash. |

**No silent fallback**: selecting a provider whose feature is not compiled or
whose API key is missing is a hard `Init` error at daemon startup (B6 posture).
Switch to `hash` in `[knowledge.embedding] provider = "hash"` for offline work.

Collections in Qdrant are namespaced by embedder model id, so switching
providers never mixes vector dimensions. See the migration path in step 4.

## 3. Ingest

The public ingest surfaces are the **Rust `Backend` API**, the **MCP
`knowledge_index` path**, and the **`tdw kg ingest` CLI command** (landed in K-E3).

Index documents via the `Backend` API (Rust):

```rust
backend.knowledge_index(KnowledgeDocument {
    id: "doc-aapl-1".to_string(),
    body: "AAPL services revenue note".to_string(),
    entity: Entity {
        entity_id: "instrument:AAPL".to_string(),
        kind: EntityKind::Instrument,
        label: "Apple".to_string(),
        aliases: vec!["AAPL".to_string()],
    },
    tags: vec!["asset:equity".to_string()],
    source: None,
    plane: None,
    as_of: None,
    mentions: vec![],
}).await?;
```

Or in batch via `Backend::knowledge_ingest_at`:

```rust
backend.knowledge_ingest_at(vec![doc1, doc2], "2026-06-11").await?;
```

Ingest is content-hash-idempotent: re-indexing the same `id` with the same body
is a no-op. Documents are co-indexed in Qdrant (vector), Meilisearch (lexical),
and the graph engine (edges).

## 4. Search and Answer

```bash
# MCP tool — hybrid semantic + lexical + graph-neighbourhood search:
# tdw.kg.search  { "query": "AAPL revenue", "limit": 5 }

# Programmatic:
let hits = backend.knowledge_search("AAPL revenue", 5).await?;
```

Results are ranked by a configurable blend of vector similarity, lexical BM25,
and graph-neighbourhood score.

The `tdw.kg.answer` answer synthesis path runs the top hits through the
configured LLM (`[model]` section); no MCP tool exists for this yet — it is
consumed programmatically via `KnowledgeRuntime`.

## Migrating from `in-memory` to `bolt` (no data loss)

When you are ready to switch from the dev default to a persistent graph, follow
these steps in order:

1. Start Memgraph:
   ```bash
   docker compose --profile full up -d memgraph
   ```
2. Update your daemon config:
   ```toml
   [knowledge.graph]
   backend  = "bolt"
   bolt_uri = "bolt://127.0.0.1:7687"
   ```
3. **Re-index all documents** — graph edges from the in-memory engine are lost
   on restart and must be rebuilt:
   ```bash
   tdw kg reindex
   ```
   `tdw kg reindex` is the **mandatory data-loss-preventing step**. Without it
   graph-neighbourhood scoring returns empty results and tag edges are missing.
4. Restart the daemon. The startup notice disappears and the bolt engine is live.

> **Note**: Switching embedder provider also requires `tdw kg reindex` because
> vector collections are namespaced per model id and old vectors are not
> readable by a new model.

## Try it: offline demo walkthrough

Run a fully offline, in-memory walkthrough of the knowledge graph in one
command — no running daemon, no API keys, no external services required:

```bash
tdw kg demo
```

The demo seeds 8 curated finance documents (instruments, companies, filings),
runs inference (supply-chain peer derivation), and walks you through five steps:

| Step | What it shows |
|------|---------------|
| Ingest | 8 fixture docs indexed; derived `supply_chain_peer` edges minted |
| Search | Hybrid search for "AAPL supply chain" |
| Why | Provenance chain for a derived edge (rule id + support) |
| Diff | Manifest diff between v1 (Jan 2026) and v2 (Apr 2026) snapshots |
| Status | Document count, derivation count, embedder model, inference version |

> **In-memory demo — data is not persisted.** The graph is discarded at exit.
> To persist data, configure a bolt backend and run `tdw kg reindex`.

Use `--json` for scripted / CI output (emits NDJSON — one JSON object per step):

```bash
# Print the search step result:
tdw kg demo --json | jq 'select(.step == "search")'

# Print all step names:
tdw kg demo --json | jq -r '.step'
```

The equivalent production API call for each step is printed alongside the demo
output so you can copy it into your own code.

## Trust-Dial: Filtering by Provenance Class (K-X3)

`tdw.kg.search` accepts an optional `provenance_classes` array that restricts
results to documents of the requested provenance class. This lets agents answer
"from human-vetted knowledge only" in a single flag.

**Available classes today (doc-index path):**

| Class | When stamped | Meaning |
|---|---|---|
| `document_ingested` | Default — all non-Finding entity kinds | Externally sourced content (news, filings, provider data). |
| `user_authored` | `EntityKind::Finding` documents | Analyst research notes and findings authored by a named user. |

**Example — user-vetted knowledge only:**

```json
{
  "query": "AAPL supply chain risk",
  "top_k": 5,
  "provenance_classes": ["user_authored"]
}
```

The response includes a `trust_scope` field that honestly reports whether
filtering was active and which classes are in scope:

```json
{
  "hits": [...],
  "trust_scope": {
    "filtered": true,
    "provenance_classes": ["user_authored"],
    "note": "results restricted to user_authored provenance class"
  }
}
```

**Omitting `provenance_classes`** (or passing an empty array) returns all
classes — behavior is identical to pre-K-X3 callers, so existing integrations
are unaffected.

**Old index points** that predate the stamp are treated as `document_ingested`
(conservative default — they are never silently excluded from a
`document_ingested`-only query).

> **Note**: `rule_derived` and `agent_proposed` are reserved class tokens for a
> future graph-channel filter pass and are not yet reachable via `tdw.kg.search`.
> The MCP schema enum is intentionally restricted to the two classes that the
> doc-index retrieval path produces today.

## Further reading

- [`docs/ops/graph-db.md`](ops/graph-db.md) — Memgraph deployment, backup,
  upgrade, conformance tests, and troubleshooting.
- [`docs/kg-tags.md`](kg-tags.md) — crate responsibilities, graph engine
  selection reference, MCP surface.
- [`docs/llm-knowledge.md`](llm-knowledge.md) — LLM adapter, embedding layer,
  knowledge runtime, eval harness, and MCP tool reference.
