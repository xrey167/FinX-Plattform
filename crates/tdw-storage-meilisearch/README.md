# tdw-storage-meilisearch

Meilisearch [`LexicalEngine`](../tdw-core/src/lib.rs) for the FinX
data-warehouse full-text-search tier.

## Purpose

Indexes documents and answers lexical (full-text) queries. Ships:

- [`InMemoryLexicalEngine`] — always available, no network. Substring
  match-counting relevance over an in-memory map, for offline tests.
- [`MeilisearchHttpEngine`] — real reqwest HTTP backend behind the `meilisearch`
  feature (Meilisearch REST API, port 7700).

## Engine trait

`LexicalEngine`:

- `index(index, docs) -> Result<()>`
- `search_text(index, query) -> Result<Vec<ScoredDoc>>`

## Default (in-memory) vs real backend

| | Type | Feature | Network |
|---|---|---|---|
| Default | `InMemoryLexicalEngine` | — (always built) | none |
| Real | `MeilisearchHttpEngine` | `meilisearch` | reqwest HTTP |

Default features list is empty; `cargo test --workspace` stays offline. Enable
the real backend with `--features meilisearch`.

## Connection / env vars

```rust
// endpoint, optional api key
let engine = MeilisearchHttpEngine::new("http://127.0.0.1:7700", None)?;
```

`index` polls `/tasks/{uid}` until the task succeeds, so a caller can immediately
follow with `search_text` without flakiness. `showRankingScore: true` populates
`ScoredDoc.score`; the `_rankingScore` field is stripped from returned doc fields.

The env-gated integration test (`tests/http_engine.rs`) reads
`TDW_MEILISEARCH_TEST_URL`.

## `TDW_PROFILE=live` behavior

In the `live` profile (post-#157) the service lexical engine is
`MeilisearchHttpEngine`, wired by
[`select_lexical_engine`](../tdw-service-api/src/app_state.rs) from:

| Env var | Meaning |
|---|---|
| `TDW_MEILI_URL` | REST endpoint (required) |
| `TDW_MEILI_API_KEY` | API key (optional) |

A missing URL or absent `real-meilisearch` feature fails the `live` boot closed.

## Quickstart (offline)

```rust
use serde_json::json;
use tdw_core::{LexicalDoc, LexicalEngine, TextQuery};
use tdw_storage_meilisearch::InMemoryLexicalEngine;

# async fn run() -> tdw_core::Result<()> {
let engine = InMemoryLexicalEngine::default();
engine.index("research", vec![LexicalDoc { id: "note-1".into(), body: "AAPL volatility note".into(), fields: json!({}) }]).await?;
let hits = engine.search_text("research", TextQuery { text: "volatility".into(), top_k: 5 }).await?;
assert_eq!(hits[0].id, "note-1");
# Ok(())
# }
```

```sh
cargo run -p tdw-storage-meilisearch --example tdw-storage-meilisearch-basic
```

See [`ARCHITECTURE.md`](ARCHITECTURE.md) and
`docs/quality/production-storage-transports.md`.
