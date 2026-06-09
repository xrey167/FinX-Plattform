# tdw-core

Foundational data-platform contracts: the provider traits, the typed
result envelope, the storage-engine ports, and the in-process provider
registry that every other crate in the workspace builds on.

`tdw-core` deliberately contains **no I/O and no provider implementations**.
It is a pure-contract crate: a small set of traits and value types that
concrete providers (`tdw-provider-*`), storage adapters, and the service
layer implement or consume. This keeps the dependency graph acyclic and lets
providers be unit-tested against fixtures without any network or database.

## What it provides

- `Fetcher<Q, D>` / `Streamer<Q, D>` — the two provider shapes (request/response
  and subscription), each with a default `fetch` / `subscribe` orchestration.
- `QueryParams` / `DataModel` — blanket marker traits that bound provider query
  and row types to `Serialize + DeserializeOwned + JsonSchema + Send + Sync`.
- `OBBject<T>` — the typed result envelope (`provider`, `endpoint`, `rows`,
  `metadata`) returned by every fetch.
- `Credentials` — the credential bag threaded through `extract_data` / `subscribe`.
- Storage-engine ports — `WriteSink`, `OlapEngine`, `RelationalEngine`,
  `VectorEngine`, `LexicalEngine`, `BlobEngine` — abstract over the concrete
  warehouses/indexes so the service layer stays storage-agnostic.
- `RegistryEntry` / `ProviderRegistry` / `ProviderKind` — provider discovery and
  duplicate-registration guarding.

## Feature flags

| Feature                    | Default | Effect |
|----------------------------|---------|--------|
| `inventory-registration`   | off     | Pulls in the optional `inventory` crate and enables `ProviderRegistry::from_inventory()`, which collects every `RegistryEntry` submitted via `inventory::submit!` across the linked binary. With the feature off, `from_inventory()` returns an empty registry and providers must be registered explicitly with `register_fetcher` / `register_streamer` / `register`. |

There are **no other features and no optional behavior** — the default build is
the full contract surface.

## Quickstart

Implement `Fetcher` for a provider that turns a JSON query into typed rows:

```rust
use async_trait::async_trait;
use bytes::Bytes;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tdw_core::{Credentials, Error, Fetcher, Result};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct Quote { symbol: String, price: f64 }

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
struct QuoteQuery { symbol: String }

struct DemoFetcher;

#[async_trait]
impl Fetcher<QuoteQuery, Quote> for DemoFetcher {
    const PROVIDER: &'static str = "demo";
    const ENDPOINT: &'static str = "equity_quote";

    fn transform_query(params: Value) -> Result<QuoteQuery> {
        let symbol = params
            .get("symbol")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidQuery("missing symbol".into()))?;
        Ok(QuoteQuery { symbol: symbol.to_uppercase() })
    }

    async fn extract_data(&self, q: &QuoteQuery, _: &Credentials) -> Result<Bytes> {
        Ok(Bytes::from(format!("{}:101.0", q.symbol))) // fixture bytes
    }

    fn transform_data(&self, _q: &QuoteQuery, raw: Bytes) -> Result<Vec<Quote>> {
        let text = String::from_utf8(raw.to_vec()).map_err(|e| Error::Provider(e.to_string()))?;
        let (symbol, price) = text.split_once(':').ok_or_else(|| Error::Provider("bad row".into()))?;
        let price = price.parse().map_err(|_| Error::Provider("bad price".into()))?;
        Ok(vec![Quote { symbol: symbol.to_string(), price }])
    }
}
```

`fetch` (the trait's provided method) runs `transform_query → extract_data →
transform_data` and wraps the rows in an `OBBject<Quote>` stamped with the
provider/endpoint constants.

A runnable, offline version of this lives in
[`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-core --example tdw_core_basic
```

## Conventions

- Providers expose their identity with a `const fn registry_entry() -> RegistryEntry`
  built from `RegistryEntry::fetcher(Self::PROVIDER, Self::ENDPOINT)` (see any
  `tdw-provider-*` crate). The registry then guards against duplicate
  `(provider, endpoint, kind)` triples.
- `OBBject::new` takes `&'static str` provider/endpoint so the envelope identity
  is always the same compile-time constants the trait declares.

## Invariants

- `#![forbid(unsafe_code)]` — no `unsafe` anywhere in the crate.
- Workspace lints deny `unwrap`, `dbg!`, and `todo!`; all fallible paths return
  the crate's `Error` enum.
- No provider implementations and no I/O live here — only contracts.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the module map and trait contracts.
