# tdw-provider-fmp

Market-data provider for **Financial Modeling Prep (FMP)**. Exposes typed
query/record models plus two `tdw_core::Fetcher` implementations: daily OHLCV
bars and income statements.

- **Vendor:** Financial Modeling Prep — REST API v3
- **Base URL:** `https://financialmodelingprep.com/api/v3`
- **Endpoints:**
  - `equity_historical` — `GET /historical-price-full/{symbol}` → `tdw_domain::MarketDataBar`
  - `income_statement` — `GET /income-statement/{symbol}` (also balance / cash-flow) → `FmpIncomeRow`
- **Auth:** API key passed as the `apikey` query parameter.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `FmpHttpHistoricalFetcher` / `FmpHttpIncomeFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off the typed models, symbol/limit validation, and the
`FmpMockHistoricalFetcher` / `FmpMockIncomeFetcher` stubs are available with no
network dependencies.

## Environment variables

| Variable           | Required | Purpose |
| ------------------ | -------- | ------- |
| `TDW_FMP_API_KEY`  | for live calls | FMP API key, appended as `apikey`. |
| `TDW_FMP_LIVE`     | no       | Set to `1` to enable the live network integration test. |

The env-var name is exported as `API_KEY_ENV`.

## Quickstart

Offline (default features):

```rust
use tdw_provider_fmp::{FmpHistoricalQuery, FmpMockHistoricalFetcher};

let query = FmpHistoricalQuery::new("aapl")?;
for bar in FmpMockHistoricalFetcher::fetch_stub(&query)? {
    println!("{} {} close={}", bar.symbol, bar.date, bar.close);
}
# Ok::<(), tdw_provider_fmp::FmpError>(())
```

Live HTTP (requires `--features http` and `TDW_FMP_API_KEY`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_fmp::FmpHttpIncomeFetcher;

let fetcher = FmpHttpIncomeFetcher::default();
let rows = fetcher
    .fetch(serde_json::json!({ "symbol": "AAPL", "statement": "income", "limit": 5 }),
           &Credentials::default())
    .await?;
```

## Example

```bash
cargo run -p tdw-provider-fmp --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider env-var conventions and feature-gating model.
