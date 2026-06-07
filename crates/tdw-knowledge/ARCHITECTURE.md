# tdw-knowledge — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`:

| Item | Role |
| --- | --- |
| `KnowledgeIndex` | The façade tying embedder + vector engine + graph + tags together. |
| `KnowledgeDocument` | `{ id, body, entity, tags }` — what you index. |
| `KnowledgeHit` | `{ id, score, entity_id, tags }` — a search result. |
| `KnowledgeError` | The error enum (embedding / storage / tag / payload / document / query). |
| `validate_document` / `validate_query` | Input hygiene. |
| `summarize_syntax` / `SymbolRef` / `SyntaxSummary` | Standalone SQL/code symbol extraction. |

## Injection seam

`KnowledgeIndex::new(embedder: Arc<dyn EmbeddingProvider>, vectors: Arc<dyn VectorEngine>)`
is the durability/backends seam: the daemon passes its shared `VectorEngine`
(which may be a real Qdrant engine) so indexed knowledge persists across the same
engine the rest of the daemon uses. `KnowledgeIndex::default()` wires the offline
`HashEmbeddingProvider` + an in-process engine, preserving deterministic offline
behaviour. The graph and tag store are always in-process.

## Per-embedder collection namespacing

`collection_name(model_id)` → `tdw_knowledge__<sanitized model id>` (non-alnum →
`_`). Two embedders of different dimension therefore map to *different*
collections and never silently corrupt one another; the backing engine's
dimension guard rejects any residual collision loudly.

## Index flow

```
KnowledgeDocument
  └▶ validate_document (id, body, entity, tags)
  └▶ embedder.embed(body) → Embedding
  └▶ graph.upsert_entity(entity)
  └▶ graph.add_relationship(entity → "document:<id>", "described_by")
  └▶ for each tag: tags.define + tags.assign
  └▶ vectors.upsert(collection, VectorPoint { id, vector, payload: {entity_id, tags} })
```

## Search flow

```
query, top_k
  └▶ validate_query (non-empty query, top_k > 0)
  └▶ embedder.embed(query) → Embedding
  └▶ vectors.search_knn(collection, { vector, top_k }) → hits
  └▶ for each hit: read payload entity_id (str) + tags (string array)
       (a malformed payload → KnowledgeError::InvalidPayloadField)
  └▶ Vec<KnowledgeHit>
```

Auxiliary queries: `active_tags(entity_id, as_of)` (tag store) and
`neighbors(entity_id)` (graph).

## Offline cassette-test design

The default index is fully offline and deterministic — the hash embedder produces
reproducible vectors and the in-process engine needs no network. Tests cover the
index→search round-trip (both via `default()` and via the `new()` injection
seam), the per-model collection namespacing, a deliberately malformed vector
payload (asserting `InvalidPayloadField`), document/query validation rejections,
and `summarize_syntax`. There is no "cassette" because there is no network call to
record; determinism comes from the offline embedder.
