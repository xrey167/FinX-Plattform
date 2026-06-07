# Architecture — tdw-storage-qdrant

## Module map

| Path | Feature | Contents |
|---|---|---|
| `src/lib.rs` | always | `InMemoryVectorEngine`, dimension/collection/point validation, dot-product k-NN, unit tests |
| `src/http_engine.rs` | `qdrant` | `QdrantHttpEngine` (reqwest REST client) |
| `tests/http_engine.rs` | `qdrant` + env | double-gated integration test |

`src/lib.rs` re-exports `QdrantHttpEngine` under `#[cfg(feature = "qdrant")]`.

## Trait contract & invariants

`tdw_core::VectorEngine`:

- **`upsert`** — rejects an empty point list; validates every point id is
  non-empty and its vector non-empty; enforces a single dimension across the
  batch **and** against any existing points in the collection. Existing ids are
  overwritten in place (upsert semantics); new ids are appended.
- **`search_knn`** — rejects an empty query vector and `top_k == 0`; errors on an
  unknown collection or a query/point dimension mismatch; scores by dot product,
  sorts descending, truncates to `top_k`.

### Dimension invariant

A collection is mono-dimensional: the dimension is fixed by the first point and
every later upsert / query vector must match it. This mirrors Qdrant's
per-collection vector size, so the in-memory engine rejects exactly what the real
engine would.

## Real-vs-stub duality design

`InMemoryVectorEngine` (always built) is the offline default; `QdrantHttpEngine`
(feature `qdrant`) is the real REST client, pulling reqwest with `rustls-tls` +
`json` and `default-features = false`. The default workspace build neither
compiles nor links the HTTP stack. The service layer selects the HTTP engine only
on the `live` path. The real engine lazily creates the collection on first upsert
from the first point's dimension; the in-memory engine simply allocates the
collection map entry.

## Env-gated integration test pattern

`tests/http_engine.rs` is **double-gated**: compiled only with `--features
qdrant`, runs only when `TDW_QDRANT_TEST_URL` is set (else early-returns with a
stderr notice).

### Docker recipe

```powershell
docker compose --profile full up -d qdrant
$env:TDW_QDRANT_TEST_URL = "http://127.0.0.1:6333"
cargo test -p tdw-storage-qdrant --features qdrant --test http_engine
docker compose --profile full down -v
```

## Migration story

None. Qdrant collections are created lazily on first upsert (dimension inferred
from the first point). No SQL migration catalog applies to the vector tier.
