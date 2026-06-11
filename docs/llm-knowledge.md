# LLM And Knowledge Intelligence

The knowledge-system (stories A1–F1) ships a fully integrated retrieval and
inference layer. The key crates are:

## Language model layer

- `tdw-llm`: the in-house `LanguageModel` trait, chat request/response types,
  usage accounting, and config-derived model selection. The `StubLanguageModel`
  provides deterministic offline responses for eval and tests.
- `tdw-llm-anthropic`: adapter for Anthropic Messages-style providers.
- `tdw-llm-openai-compat`: adapter for OpenAI-compatible providers.
  Neither adapter makes network calls in tests.

## Embedding layer

- `tdw-embed`: `EmbeddingProvider` trait and `Embedding` type.
- `tdw-embed-local`: `HashEmbeddingProvider` (deterministic, offline; the
  default for dev/test) and the optional `candle`-based local BERT stack
  (behind the `model` feature).
- `tdw-embed-openai`: real OpenAI HTTP embedder (behind the `openai` feature).
- `tdw-embed-google`: real Google/Gemini HTTP embedder (behind the `google`
  feature).

Provider selection is config-driven via `[knowledge.embedding]` in the daemon
TOML. No silent fallback: a requested provider without its key is a hard `Init`
error at daemon startup (B6 posture).

## Knowledge and retrieval layer

- `tdw-knowledge`: `KnowledgeRuntime` (hybrid Retriever + graph/tag handles +
  version triple) and `KnowledgeIndexer` (content-hash idempotency,
  rule-driven auto-tagging, lexical + graph co-index, semantic dedup via
  embedding similarity). `collection_name(model_id)` derives the stable Qdrant
  collection name from the active embedder.
- `tdw-retrieve`: hybrid `Retriever` combining vector (Qdrant), lexical
  (Meilisearch), and graph (BFS neighborhood) results with configurable weights.
  Wired into `KnowledgeRuntime` so all three lenses are queried on every
  `search` call.
- `tdw-infer`: inference rules and version stamps (`KnowledgeVersions`).
- `tdw-storage-graph`: `GraphEngine` trait, `InMemoryGraphEngine` (always
  compiled), and `BoltGraphEngine` (behind the `bolt` feature; targets
  Memgraph/Neo4j over the Bolt protocol).

## Eval harness

- `tdw-eval-runner`: runs eval cases through a `LanguageModel`. The daemon
  injects `StubLanguageModel` by default so `run_eval` never hits the network
  in CI. Real model injection is possible by building `tdw-backend` with the
  `openai` or `google` feature and setting the appropriate API key environment
  variable.

## Unified daemon wiring (F1)

`tdw-backend::data::Backend` hosts the full knowledge system:

- `KnowledgeRuntime` with the hybrid Retriever, graph engine, and lexical
  engine — all sharing the same `EmbeddingProvider` and `VectorEngine` as the
  daemon composition root.
- `KnowledgeIndexer` constructed on demand (shares the same `Arc`s — no
  duplication of large state).
- `RetrievalFeedbackStore` for the write-back loop.
- Graph engine selected at startup from `[knowledge.graph]` config: `bolt`
  (production) or `in-memory` (dev/test). Hard `Init` error on unreachable
  Bolt — no silent fallback.

In `Surfaces::Both` mode the `KnowledgeRuntime` and `RetrievalFeedbackStore`
handles are injected directly into the embedded MCP server via
`McpServer::with_knowledge` / `McpServer::with_feedback_store`, so knowledge
read/write tools operate without a loopback round-trip to the daemon.

## MCP tools

The following MCP tools are live when a `KnowledgeRuntime` is attached:

| Tool | Description |
|---|---|
| `tdw.kg.search` | Hybrid semantic + lexical + graph-neighborhood search. |
| `tdw.kg.get_entity` | Retrieve a single entity by ID with its tag set. |
| `tdw.kg.neighbors` | BFS neighborhood traversal from an entity. |
| `tdw.kg.infer` | Run inference rules against the graph. |
| `tdw.kg.feedback` | Record retrieval feedback signal (write gate: requires `write_tools_enabled = true`; off by default for agent surfaces). |
