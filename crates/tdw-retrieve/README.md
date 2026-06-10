# tdw-retrieve

Hybrid retrieval for the knowledge system (overhaul slice B4).

`Retriever` fans a `KnowledgeQuery` out across up to three channels — vector
KNN, lexical full-text, and the temporal tag index — fuses the ranked lists
with pure Reciprocal Rank Fusion (`rrf_fuse`, k=60, rank-only, deterministic
id tie-break), optionally expands the top fused hits through the knowledge
graph with per-hop score decay, and returns `RetrievedHit`s that explain
themselves: which channels found them at which rank, which (subsumption-
expanded) query tags matched, and the graph path that reached an expanded hit.

Design points:

- **Channels are optional by construction.** A bare `Retriever::new` is
  vector-only and behaves exactly like the pre-B4 `KnowledgeIndex::search`,
  which now delegates here. `with_lexical` / `with_tags` / `with_graph`
  attach the other channels.
- **Trait-only substrate.** The lib depends on `VectorEngine`,
  `LexicalEngine`, `GraphEngine` (tdw-core), `EmbeddingProvider` (tdw-embed),
  and `TagEngine` (tdw-tags); real backends (Qdrant, Meilisearch, Memgraph)
  appear only in dev-dependencies for the end-to-end tests.
- **Temporal queries are leakage-safe by construction.** `as_of` is a
  tag-date (`YYYY-MM-DD`) mapped onto normalized timestamps for the payload
  filter; documents dated later — and documents without a date — are
  structurally invisible. The tag channel runs only on temporal queries, and
  the graph-derived paths (tag channel, graph expansion) re-apply the same
  contract through document-node props (`as_of`, `plane`) plus the entity
  node's kind — no channel is a way around the payload gate. Ingestion (B5)
  stamps those props when it writes `described_by` edges.
- **Hostile-query bounds.** `top_k <= 256` and expansion `per_hit_limit <= 64`
  are hard `try_new` errors; hops are capped at 3 and expansion seeds at 8.
- **Failures are loud.** A failing channel surfaces its engine's error;
  nothing degrades silently.

Run the tests:

```text
cargo test --target-dir target -p tdw-retrieve
```
