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

Index documents through the daemon's knowledge API or the MCP tool:

```bash
# Via tdw-cli (if wired):
tdw kg index --id "doc-aapl-1" --body "AAPL services revenue note" \
  --entity "instrument:AAPL" --tags "asset:equity"

# Or programmatically via the Backend API (Rust):
backend.knowledge_index(KnowledgeDocument {
    id: "doc-aapl-1".to_string(),
    body: "AAPL services revenue note".to_string(),
    entity: Entity { entity_id: "instrument:AAPL".to_string(), .. },
    tags: vec!["asset:equity".to_string()],
    ..Default::default()
}).await?;
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

## Further reading

- [`docs/ops/graph-db.md`](ops/graph-db.md) — Memgraph deployment, backup,
  upgrade, conformance tests, and troubleshooting.
- [`docs/kg-tags.md`](kg-tags.md) — crate responsibilities, graph engine
  selection reference, MCP surface.
- [`docs/llm-knowledge.md`](llm-knowledge.md) — LLM adapter, embedding layer,
  knowledge runtime, eval harness, and MCP tool reference.
