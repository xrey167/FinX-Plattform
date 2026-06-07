# tdw-provider-cboe

CBOE (Cboe Global Markets) delayed options and US-index provider for the TDW
platform. Wraps the public CBOE CDN API (`https://cdn.cboe.com/api/global`) and
exposes two read endpoints as `tdw_core::Fetcher` implementations.

| Endpoint              | Fetcher                    | Path                                  | Row model              |
| --------------------- | -------------------------- | ------------------------------------- | ---------------------- |
| Delayed options chain | `CboeHttpOptionsFetcher`   | `GET /delayed_quotes/options/{sym}`   | `CboeOptionContract`   |
| US-index quote        | `CboeHttpIndexFetcher`     | `GET /us_indices/quotes/{index}`      | `CboeIndexQuote`       |

**No API key is required** — CBOE's CDN endpoints are unauthenticated. The crate
compiles and tests offline by default: query structs, request-path helpers,
domain models, and `stub_*` helpers are always available, while the
network-backed fetchers and `reqwest` dependency only exist under the `http`
feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-cboe --features http
```

## Environment variables

| Variable              | Required for           | Purpose                                                       |
| --------------------- | ---------------------- | ------------------------------------------------------------ |
| _(none — no API key)_ | —                      | CBOE public CDN endpoints are unauthenticated.               |
| `TDW_CBOE_LIVE=1`     | live integration tests | Opt-in gate; without it the live tests skip so CI is offline. |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_cboe::CboeHttpIndexFetcher;

# async fn run() -> tdw_core::Result<()> {
let fetcher = CboeHttpIndexFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "index": "VIX" }), &Credentials::default())
    .await?;
for quote in obb.rows {
    println!("{} price={} change={}", quote.symbol, quote.price, quote.change);
}
# Ok(())
# }
```

The options fetcher takes `symbol` (alias `ticker`); the index fetcher takes
`index` (alias `symbol`, uppercase ASCII letters only).

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-cboe --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
