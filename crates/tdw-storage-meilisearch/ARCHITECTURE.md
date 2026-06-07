# Architecture — tdw-storage-meilisearch

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `InMemoryLexicalEngine`, index/doc/query validation, substring relevance scoring, unit tests |
| `src/http_engine.rs` | `meilisearch` | `MeilisearchHttpEngine` (reqwest REST client) |
| `tests/http_engine.rs` | `meilisearch` + env | double-gated integration test |

`src/lib.rs` re-exports `MeilisearchHttpEngine` under `#[cfg(feature = "meilisearch")]`.

## Trait contract & invariants

`tdw_core::LexicalEngine`:

- **`index`** — validates the index name and each doc id is non-empty; existing
  ids are overwritten in place (upsert), new ids appended.
- **`search_text`** — validates the index name, non-empty query text and
  `top_k > 0`; errors on an unknown index. The in-memory engine scores by
  case-insensitive substring match count, drops zero-score docs, sorts descending,
  truncates to `top_k`.

### Read-after-write invariant (real engine)

Meilisearch indexing is asynchronous. `MeilisearchHttpEngine::index` polls
`/tasks/{uid}` until the task reaches `succeeded` before returning, so a caller
can `search_text` immediately and deterministically — matching the
synchronous-feeling contract the in-memory engine provides for free.

## Real-vs-stub duality design

`InMemoryLexicalEngine` (always built) is the offline default; the reqwest HTTP
engine is opt-in behind the `meilisearch` feature (which also pulls `serde` /
`serde_json` for request/response shaping). The default workspace build links no
HTTP stack. The service layer selects the HTTP engine only on the `live` path.

The relevance models differ deliberately: the in-memory engine uses a simple
match-count score (good enough to assert ordering offline), while the real engine
returns Meilisearch's ranking score via `showRankingScore`. The trait surface
(`ScoredDoc { id, score, fields }`) is identical, so callers are agnostic.

## Env-gated integration test pattern

`tests/http_engine.rs` is **double-gated**: compiled only with `--features
meilisearch`, runs only when `TDW_MEILISEARCH_TEST_URL` is set (else
early-returns with a stderr notice).

### Docker recipe

```powershell
docker compose --profile full up -d meilisearch
$env:TDW_MEILISEARCH_TEST_URL = "http://127.0.0.1:7700"
cargo test -p tdw-storage-meilisearch --features meilisearch --test http_engine
docker compose --profile full down -v
```

## Migration story

None. Meilisearch indexes are created on demand (first `index` call); there is no
SQL migration catalog for the lexical tier.
