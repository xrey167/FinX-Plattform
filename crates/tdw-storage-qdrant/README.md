# tdw-storage-qdrant

Qdrant [`VectorEngine`](../tdw-core/src/lib.rs) for the FinX data-warehouse
vector-search tier.

## Purpose

Stores embedding vectors with JSON payloads and answers k-NN queries. Ships:

- [`InMemoryVectorEngine`] — always available, no network. Brute-force dot-product
  k-NN over an in-memory map, with dimension validation, for offline tests.
- [`QdrantHttpEngine`] — real reqwest HTTP backend behind the `qdrant` feature
  (Qdrant REST API, port 6333).

## Engine trait

`VectorEngine`:

- `upsert(collection, points) -> Result<()>`
- `search_knn(collection, query) -> Result<Vec<ScoredPoint>>`

## Default (in-memory) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `InMemoryVectorEngine` | — (always built) | none |
| Real | `QdrantHttpEngine` | `qdrant` | reqwest HTTP (rustls) |

Default features list is empty; `cargo test --workspace` stays offline. Enable
the real backend with `--features qdrant`.

## Connection / env vars

```rust
// endpoint, optional api key
let engine = QdrantHttpEngine::new("http://127.0.0.1:6333", None)?;
```

The HTTP engine lazily auto-creates a collection on first upsert using the first
point's vector dimension. Point IDs must be unsigned integers or UUIDs (a Qdrant
constraint); arbitrary string-ID normalization is a follow-up.

The env-gated integration test (`tests/http_engine.rs`) reads
`TDW_QDRANT_TEST_URL`.

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the service vector engine is
`QdrantHttpEngine`, wired by
[`select_vector_engine`](../tdw-service-api/src/app_state.rs) from:

| Env var | Meaning |
|---|---|
| `TDW_QDRANT_URL` | REST endpoint (required) |
| `TDW_QDRANT_API_KEY` | API key (optional) |

A missing URL or absent `real-qdrant` feature fails the `live` boot closed.

## Quickstart (offline)

```rust
use serde_json::json;
use tdw_core::{VectorEngine, VectorPoint, VectorQuery};
use tdw_storage_qdrant::InMemoryVectorEngine;

# async fn run() -> tdw_core::Result<()> {
let engine = InMemoryVectorEngine::default();
engine.upsert("research", vec![VectorPoint { id: "a".into(), vector: vec![1.0, 0.0], payload: json!({}) }]).await?;
let hits = engine.search_knn("research", VectorQuery::knn(vec![1.0, 0.0], 1)).await?;
assert_eq!(hits[0].id, "a");
# Ok(())
# }
```

```sh
cargo run -p tdw-storage-qdrant --example tdw-storage-qdrant-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and
`docs/quality/production-storage-transports.md`.
