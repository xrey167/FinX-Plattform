# tdw-knowledge

Document knowledge index: embeds documents, stores their vectors, links their
entities in a knowledge graph, and tags them — then answers semantic search.

## Purpose

[`KnowledgeIndex`] ties together four capabilities behind one façade:

- an [`tdw_embed::EmbeddingProvider`] (default: the offline `HashEmbeddingProvider`),
- a [`tdw_core::VectorEngine`] (default: an in-process engine),
- a `tdw_kg::KnowledgeGraph` (entities + `described_by` edges),
- a `tdw_tags::TagStore` (per-entity tag assignments).

`index_document` embeds the body, upserts the entity + graph edge, defines/assigns
tags, and stores the vector with an `{ entity_id, tags }` payload. `search` embeds
the query and returns scored [`KnowledgeHit`]s. The vector collection is
namespaced per embedder `model_id`, so two embedders of different dimension never
share a collection.

There is also a small standalone helper, [`summarize_syntax`], that extracts
table/function symbols from SQL/Rust-ish text.

## Feature flags

None. Dependencies: `serde`, `serde_json`, `tdw-core`, `tdw-embed`,
`tdw-embed-local`, `tdw-kg`, `tdw-storage-qdrant`, `tdw-tags`, `thiserror`
(+ `tokio` as a dev-dependency).

## Environment variables

None read directly. A production deployment injects a real `VectorEngine` (e.g. a
Qdrant engine, configured by the daemon) via [`KnowledgeIndex::new`]; the default
is fully offline.

## Quickstart

```rust
use tdw_knowledge::{KnowledgeDocument, KnowledgeIndex};
use tdw_kg::{Entity, EntityKind};

# async fn run() -> tdw_knowledge::Result<()> {
let mut index = KnowledgeIndex::default(); // offline hash embedder + in-process vectors
index.index_document(KnowledgeDocument {
    id: "doc-1".to_string(),
    body: "AAPL equity momentum research".to_string(),
    entity: Entity {
        entity_id: "instrument:AAPL".to_string(),
        kind: EntityKind::Instrument,
        label: "Apple".to_string(),
        aliases: vec!["AAPL".to_string()],
    },
    tags: vec!["asset:equity".to_string()],
}).await?;

let hits = index.search("AAPL momentum", 1).await?;
assert_eq!(hits[0].id, "doc-1");
# Ok(()) }
```

## Example

```text
cargo run --example tdw_knowledge_basic -p tdw-knowledge
```

`examples/basic.rs` indexes a document and searches it back through the default
offline index — deterministic, no network.
