# tdw-provider-trading-economics

Trading Economics data provider for the TDW (Trading Data Warehouse) platform.

Exposes offline query/validation types, deterministic offline stub fetchers,
and two real HTTP `Fetcher`s for the **Trading Economics** API
(`https://api.tradingeconomics.com`): the economic calendar and country
indicators. Network access is feature-gated so the workspace test set runs
fully offline.

## What it provides

- `TradingEconomicsCalendarQuery` / `TradingEconomicsIndicatorQuery` —
  validated query types.
- `TradingEconomicsCalendarEvent` / `TradingEconomicsIndicatorRow` — data
  models.
- `TradingEconomicsMockCalendarFetcher` / `TradingEconomicsMockIndicatorFetcher`
  — offline stub fetchers (`fetch_stub`) that never touch the network.
- `TradingEconomicsHttpCalendarFetcher` / `TradingEconomicsHttpIndicatorFetcher`
  — real HTTP fetchers (under `http`).

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `http_fetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off, the query types, data models, error enum, and the mock
fetchers are all still available.

## Environment variables

| Variable | Purpose |
| -------- | ------- |
| `TDW_TRADING_ECONOMICS_API_KEY` | Trading Economics API key, appended to live requests. Required for live calls. Exported as `API_KEY_ENV`. |

## Quickstart

```rust
use tdw_provider_trading_economics::{
    TradingEconomicsCalendarQuery, TradingEconomicsMockCalendarFetcher,
};

let query = TradingEconomicsCalendarQuery::new(3)?; // importance >= 3
let events = TradingEconomicsMockCalendarFetcher::fetch_stub(&query)?;
assert_eq!(events[0].importance, 3);
# Ok::<(), tdw_provider_trading_economics::TradingEconomicsError>(())
```

With the `http` feature, drive the real fetcher:

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_trading_economics::TradingEconomicsHttpCalendarFetcher;

std::env::set_var("TDW_TRADING_ECONOMICS_API_KEY", "…");
let fetcher = TradingEconomicsHttpCalendarFetcher::default();
let obb = fetcher
    .fetch(serde_json::json!({ "importance_min": 3 }), &Credentials::default())
    .await?;
println!("{} events", obb.rows.len());
```

## Example

```bash
cargo run -p tdw-provider-trading-economics --example basic
```

See [`examples/basic.rs`](examples/basic.rs) — uses the offline stub fetchers,
so it needs no feature flags or network.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
