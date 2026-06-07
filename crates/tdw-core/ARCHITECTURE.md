# tdw-core architecture

`tdw-core` is a single-module contract crate (`src/lib.rs`). It defines the
traits and value types that bound the whole platform's provider and storage
layers. Nothing here performs I/O; every concrete behavior is supplied by an
implementor crate.

## Module map

Everything is declared in `src/lib.rs`, grouped as:

| Group                | Items |
|----------------------|-------|
| Error type           | `Error` (`InvalidQuery` / `Provider` / `Storage` / `Registry`), `Result<T>` alias |
| Marker traits        | `QueryParams`, `DataModel` (blanket impls) |
| Result envelope      | `OBBject<T>` (`new`, `with_metadata`) |
| Credentials          | `Credentials` |
| Provider traits      | `Fetcher<Q, D>`, `Streamer<Q, D>`, `DataStream<T>`, `ProgressStream<T>`, `ProgressOrResult<T>` |
| Storage ports        | `WriteSink<T>`, `OlapEngine`, `RelationalEngine`, `VectorEngine`, `LexicalEngine`, `BlobEngine`, plus their value types (`VectorPoint`, `VectorQuery`, `ScoredPoint`, `LexicalDoc`, `TextQuery`, `ScoredDoc`, `WriteReceipt`, `HealthStatus`) |
| Registry             | `ProviderKind`, `RegistryEntry`, `ProviderRegistry` |

## Core trait contracts

### `QueryParams` / `DataModel`

```rust
pub trait QueryParams: Clone + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static {}
pub trait DataModel:  Clone + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static {}
```

Both are **blanket-implemented** for any type meeting the bounds. They exist to
give a single name to "a type that can cross the provider boundary": it must be
cloneable, JSON-serializable in both directions, JSON-schema describable (for
contract export), thread-safe, and `'static`. Providers never implement these
directly — they just derive `Serialize, Deserialize, JsonSchema` on their query
and row structs.

### `Fetcher<Q, D>`

```rust
const PROVIDER: &'static str;
const ENDPOINT: &'static str;

fn transform_query(params: Value) -> Result<Q>;                       // JSON -> typed query (assoc, no self)
async fn extract_data(&self, query: &Q, creds: &Credentials) -> Result<Bytes>;  // typed query -> raw bytes (I/O)
fn transform_data(&self, query: &Q, raw: Bytes) -> Result<Vec<D>>;   // raw bytes -> typed rows

async fn fetch(&self, params: Value, creds: &Credentials) -> Result<OBBject<D>>; // provided
```

The three-stage **transform / extract / transform** split is the central
contract:

1. `transform_query` is an *associated* function (no `&self`) — it validates and
   normalizes the untyped JSON request into the provider's `Q` before any work
   is done. This is the place to reject bad input (`Error::InvalidQuery`).
2. `extract_data` is the only `async` / I/O stage; it returns opaque `Bytes`
   (an HTTP body, a file slice, a fixture) so transport and parsing stay
   separate.
3. `transform_data` parses those bytes into `Vec<D>`.

The provided `fetch` composes all three and wraps the result in
`OBBject::new(rows, Self::PROVIDER, Self::ENDPOINT)`, so the envelope identity is
always the compile-time constants. Implementors normally override only the three
stages and inherit `fetch`.

### `Streamer<Q, D>`

```rust
const PROVIDER: &'static str;
const ENDPOINT: &'static str;
async fn subscribe(&self, query: Q, creds: &Credentials) -> Result<DataStream<D>>;
async fn snapshot(&self, query: &Q, creds: &Credentials) -> Result<Vec<D>>;
async fn checkpoint(&self, _seq: u64) -> Result<()> { Ok(()) }       // provided no-op
```

The subscription counterpart of `Fetcher`. `subscribe` yields a
`DataStream<D> = Pin<Box<dyn Stream<Item = Result<D>> + Send>>`; `snapshot`
gives a point-in-time backfill; `checkpoint` is an optional durability hook that
defaults to a no-op.

### `OBBject<T>`

The uniform result envelope: `provider`, `endpoint`, `rows: Vec<T>`, and a
`BTreeMap<String, Value>` of metadata (ordered for deterministic serialization).
`new` takes `&'static str` identifiers; `with_metadata` is a builder. It is
`Serialize`/`Deserialize` with a `bound(deserialize = "T: DeserializeOwned")` so
it round-trips for any `DataModel`.

### `RegistryEntry` / `ProviderRegistry`

```rust
pub struct RegistryEntry { provider: &'static str, endpoint: &'static str, kind: ProviderKind }
RegistryEntry::fetcher(provider, endpoint)   // const
RegistryEntry::streamer(provider, endpoint)  // const
```

`ProviderRegistry` holds a `Vec<RegistryEntry>` and enforces that a
`(provider, endpoint, kind)` triple is registered **at most once** —
`register` returns `Error::Registry` on a duplicate. Convenience methods
`register_fetcher::<F, Q, D>()` / `register_streamer::<S, Q, D>()` derive the
entry from a trait impl's `PROVIDER`/`ENDPOINT` constants. `resolve` / `contains`
look entries up; `entries()` exposes the slice.

`ProviderRegistry::from_inventory()` is the discovery seam: with the
`inventory-registration` feature it collects every `RegistryEntry` submitted via
`inventory::submit!` across the linked binary (failing on duplicates); without
the feature it returns an empty registry, and callers register explicitly.

### Registering a provider (no macro)

> **Note on the `provider_fetcher_struct!` macro.** Some planning notes reference
> a `provider_fetcher_struct!` declarative macro. **No such macro exists in this
> crate** (or anywhere in the workspace at the time of writing). Providers are
> plain structs that `impl Fetcher` and expose a `const fn registry_entry() ->
> RegistryEntry`. The example below mirrors the real `tdw-provider-fileset`
> pattern. If a macro is later added, this section should be updated to document
> it; until then, do not reference an API that does not exist.

```rust
struct DemoFetcher;
impl DemoFetcher {
    const fn registry_entry() -> tdw_core::RegistryEntry {
        tdw_core::RegistryEntry::fetcher(
            <Self as Fetcher<_, _>>::PROVIDER,
            <Self as Fetcher<_, _>>::ENDPOINT,
        )
    }
}
```

## Storage ports

`WriteSink`, `OlapEngine`, `RelationalEngine`, `VectorEngine`, `LexicalEngine`,
and `BlobEngine` are `async_trait` object-safe ports. They let the ingestion and
query layers depend on capabilities ("write a batch", "run KNN search") rather
than on ClickHouse / Postgres / Qdrant / Meilisearch / S3 concretely. The
concrete adapters live in their own crates and implement these traits.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]` at the crate root.
- **No `unwrap` / `dbg!` / `todo!`.** Workspace clippy lints deny them; every
  fallible path returns `Result<T, Error>`.
- **Clean-room.** No third-party data-vendor names, no derived branding; the
  contracts are written from first principles. (The repo's
  `xtask clean-room-audit` scans all `.rs`/`.toml` for forbidden strings.)
- **Pure contracts.** No I/O, no provider logic, no storage drivers — those are
  implemented in dependent crates so the dependency graph stays acyclic.
- **Compile-time identity.** Provider/endpoint identifiers are `&'static str`
  constants on the trait, so an `OBBject` can never be stamped with a runtime
  string that disagrees with the registry entry.
