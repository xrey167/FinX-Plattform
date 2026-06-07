# tdw-provider-tradier

Tradier brokerage data provider for the TDW (Trading Data Warehouse) platform.

Exposes offline query/validation types plus two real HTTP `Fetcher`s for the
**Tradier** API (`https://api.tradier.com/v1`): real-time equity quotes and
options chains. Network access is feature-gated so the workspace test set runs
fully offline.

## What it provides

- `TradierQuoteQuery` / `TradierOptionsQuery` — validated query types.
- `TradierHttpQuoteFetcher` — `GET /markets/quotes` → `Quote` rows.
- `TradierHttpOptionsFetcher` — `GET /markets/options/chains` →
  `EquityHistoricalData` rows.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `http_fetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off, only the query/validation types, data models, and error enum
compile.

## Environment variables

| Variable | Purpose |
| -------- | ------- |
| `TDW_TRADIER_API_KEY` | Tradier bearer token, sent as `Authorization: Bearer …`. Required for live calls. Exported as `API_KEY_ENV`. |

## Quickstart

```rust
use tdw_provider_tradier::{TradierOptionsQuery, TradierQuoteQuery};

let quote = TradierQuoteQuery::new("aapl")?;
assert_eq!(quote.symbol, "AAPL");

let opts = TradierOptionsQuery::new("AAPL", "2024-01-19")?;
assert_eq!(opts.expiration, "2024-01-19");
# Ok::<(), tdw_provider_tradier::TradierProviderError>(())
```

With the `http` feature:

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_tradier::TradierHttpQuoteFetcher;

std::env::set_var("TDW_TRADIER_API_KEY", "…");
let fetcher = TradierHttpQuoteFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "symbol": "AAPL" }), &Credentials::default())
    .await?;
println!("{} quotes", obb.rows.len());
```

## Example

```bash
cargo run -p tdw-provider-tradier --example basic --features http
```

See [`examples/basic.rs`](examples/basic.rs) — runs `transform_data` against
inline Tradier fixtures, no network or token required.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
