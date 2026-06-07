# tdw-provider-velodata

Velodata (Velo) crypto-derivatives data provider for the TDW (Trading Data
Warehouse) platform.

Exposes offline query/validation types plus three real HTTP `Fetcher`s for the
**Velo** API (`https://api.velo.xyz/v1`): perpetual funding rates, aggregated
liquidations, and aggregated open interest. Network access is feature-gated so
the workspace test set runs fully offline.

## What it provides

- `VelodataFundingQuery` / `VelodataLiquidationsQuery` / `VelodataOiQuery` —
  validated query types.
- `FundingRate` / `AggregatedLiquidation` / `AggregatedOi` — data models.
- `VelodataHttpFundingFetcher` — `GET /funding/rates` → `FundingRate`.
- `VelodataHttpLiquidationsFetcher` — `GET /liquidations/aggregated` →
  `AggregatedLiquidation`.
- `VelodataHttpOiFetcher` — `GET /oi/aggregated` → `AggregatedOi`.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `http_fetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off, only the query/validation types, data models, and error enum
compile.

## Environment variables

| Variable | Purpose |
| -------- | ------- |
| `TDW_VELODATA_API_KEY` | Velo API key, sent as the `X-API-KEY` header. Required for live calls. Exported as `API_KEY_ENV`; the header name is exported as `API_KEY_HEADER`. |

## Quickstart

```rust
use tdw_provider_velodata::{VelodataFundingQuery, VelodataOiQuery};

let funding = VelodataFundingQuery::new("binance", "BTCUSDT", 100)?;
assert_eq!(funding.exchange, "binance");

let oi = VelodataOiQuery::new("BTCUSDT", 50)?;
assert_eq!(oi.limit, 50);
# Ok::<(), tdw_provider_velodata::VelodataProviderError>(())
```

With the `http` feature:

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_velodata::VelodataHttpFundingFetcher;

std::env::set_var("TDW_VELODATA_API_KEY", "…");
let fetcher = VelodataHttpFundingFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({ "exchange": "binance", "symbol": "BTCUSDT", "limit": 100 }),
        &Credentials::default(),
    )
    .await?;
println!("{} funding samples", obb.rows.len());
```

## Example

```bash
cargo run -p tdw-provider-velodata --example basic --features http
```

See [`examples/basic.rs`](examples/basic.rs) — runs `transform_data` against
an inline Velo fixture, no network or key required.

## Configuration

Provider registration and feature-gate conventions are documented in
[`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
