# tdw-provider-tiingo

Tiingo data provider for the TDW (Trading Data Warehouse) platform.

Exposes offline query/validation types plus two real HTTP `Fetcher`s for the
**Tiingo** API (`https://api.tiingo.com/tiingo`): daily historical prices and
the news feed. Network access is feature-gated so the workspace test set runs
fully offline by default.

## What it provides

- `TiingoHistoricalQuery` / `TiingoNewsQuery` — validated query types.
- `TiingoHttpHistoricalFetcher` — `GET /daily/{symbol}/prices` →
  `MarketDataBar` rows.
- `TiingoHttpNewsFetcher` — `GET /news` → `TiingoNewsArticle` rows.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `http_fetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `serde_json`, `tdw-core`, `tdw-domain`. |

With `http` off, only the query/validation types and error enum compile.

## Environment variables

| Variable | Purpose |
| -------- | ------- |
| `TDW_TIINGO_API_KEY` | Tiingo API token, read at runtime by `extract_data` and appended as the `token` query parameter. Required for live calls. |
| `TDW_TIINGO_LIVE=1`  | Opt-in switch for the live integration test. Unset → live test skipped. |

The token constant is exported as `API_KEY_ENV`.

## Quickstart

```rust
use tdw_provider_tiingo::{TiingoHistoricalQuery, TiingoNewsQuery};

let hist = TiingoHistoricalQuery::new("aapl")?;
assert_eq!(hist.symbol, "AAPL");

let news = TiingoNewsQuery::new(&["AAPL", "MSFT"])?;
assert_eq!(news.tickers.len(), 2);
# Ok::<(), tdw_provider_tiingo::TiingoProviderError>(())
```

With the `http` feature:

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_tiingo::TiingoHttpHistoricalFetcher;

std::env::set_var("TDW_TIINGO_API_KEY", "…");
let fetcher = TiingoHttpHistoricalFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "symbol": "AAPL" }), &Credentials::default())
    .await?;
println!("{} bars", obb.rows.len());
```

## Example

```bash
cargo run -p tdw-provider-tiingo --example basic --features http
```

See [`examples/basic.rs`](examples/basic.rs) — runs `transform_data` against
an inline Tiingo fixture, no network or token required.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
