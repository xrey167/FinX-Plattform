# tdw-embed-local — Architecture

## Module map

Single-file crate (`src/lib.rs`), `#![forbid(unsafe_code)]`. Exposes
[`HashEmbeddingProvider`] only.

## Trait contract

```rust
#[async_trait::async_trait]
impl EmbeddingProvider for HashEmbeddingProvider {
    fn model_id(&self) -> &str;
    async fn embed(&self, text: &str) -> Result<Embedding>;
}
```

It implements `tdw_embed::EmbeddingProvider`. The `async fn` does no I/O — it
returns immediately with a computed vector — but stays `async` to satisfy the
trait so it is a drop-in for network-backed providers.

## Construction

| Constructor | Result |
| --- | --- |
| `HashEmbeddingProvider::default()` | `model_id = "local-hash-8"`, `dimensions = 8`. |
| `HashEmbeddingProvider::new(model_id, dimensions)` | Validates: non-empty model id (`EmptyModelId`), `dimensions > 0` (`InvalidDimensions`). |

## The hash embedding

`embed(text)`:

1. Reject empty/whitespace input → `EmbeddingError::EmptyInput`.
2. Allocate `vec![0.0_f32; dimensions]`.
3. For each input byte at `index`, add `byte / 255.0` to slot
   `index % dimensions`.
4. Return `Embedding { model_id, vector }`.

Properties:

- **Deterministic** — a pure function of `(dimensions, text)`; identical input
  produces a bit-identical vector. This is what makes the offline knowledge tests
  reproducible.
- **Fixed dimension** — every vector for one provider has the same length, so a
  vector engine's dimension guard is satisfied.
- **Finite** — all components are finite, so `validate_embedding` always passes.

It carries no semantic meaning; nearby vectors do not imply nearby meaning. Its
job is determinism, not relevance.

## Offline / cassette-test design

There is nothing to cassette — the provider never touches the network. The unit
test (`hash_embedding_is_deterministic`) asserts the same text twice yields the
same vector and that construction validation rejects bad inputs. The whole crate
is the "offline default" half of the embedding story.
