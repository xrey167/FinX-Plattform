# tdw-provider-yahoo

Yahoo Finance data provider for the TDW (Trading Data Warehouse) platform.

Provides two `Fetcher`s for Yahoo equity historical data:

- `YahooEquityHistoricalFetcher` — **always compiled**, fully offline. Its
  `extract_data` returns a deterministic synthetic bar, so the whole
  `Fetcher` pipeline runs with no network.
- `YahooHttpEquityHistoricalFetcher` — **`http`-gated**, talks to Yahoo's v8
  chart endpoint (`https://query1.finance.yahoo.com`) via `reqwest`.

Both share query validation with `tdw-provider-fileset`
(`EquityHistoricalQuery`) so symbols are normalised consistently across the
provider surface.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `http_fetcher` (the real chart-API fetcher) and pulls in `reqwest` + `tokio`. |

The offline `YahooEquityHistoricalFetcher`, query types, and the JSON
parsing helpers are available in **both** builds. Note that, unlike the other
providers, `tdw-core` / `tdw-domain` / `bytes` / `serde_json` are *not*
gated — only the live HTTP path is.

## Environment variables

| Variable | Purpose |
| -------- | ------- |
| `TDW_YAHOO_LIVE=1` | Opt-in switch read by the integration test to perform a real network call against Yahoo's chart API. Unset → the live test is skipped. |

No API key: Yahoo's delayed equity chart endpoint is unauthenticated.

## Quickstart

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_yahoo::YahooEquityHistoricalFetcher;

// Offline fetcher: full pipeline, no network, no feature flag.
let fetcher = YahooEquityHistoricalFetcher;
let obb = fetcher
    .fetch(serde_json::json!({ "symbol": "AAPL" }), &Credentials::default())
    .await?;
assert_eq!(obb.rows[0].symbol, "AAPL");
# Ok::<(), tdw_core::Error>(())
```

With the `http` feature, swap in the real chart fetcher:

```rust,ignore
use tdw_provider_yahoo::YahooHttpEquityHistoricalFetcher;

let fetcher = YahooHttpEquityHistoricalFetcher::default()
    .with_interval("1d")
    .with_range("5d");
```

## Example

```bash
cargo run -p tdw-provider-yahoo --example basic
```

See [`examples/basic.rs`](examples/basic.rs) — drives the offline fetcher
through `transform_query` → `extract_data` → `transform_data`, no network or
feature flags required.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
