# tdw-provider-finra

Market-data provider for the **FINRA Market Data API**. Exposes typed
query/record models plus two `tdw_core::Fetcher` implementations for
consolidated short interest and the weekly OTC market summary.

- **Vendor:** FINRA — public Market Data API
- **Base URL:** `https://api.finra.org/data/group`
- **Endpoints:**
  - `short_interest` — `GET /OTCMarket/block/CONSOLIDATEDSHORTINTERESTdata`
  - `otc_summary` — `GET /OTCMarket/block/WEEKLYSUMMARYdata`
- **Auth:** none (public endpoints)
- **Wire format:** pipe-delimited plain text, **not** JSON.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `FinraShortInterestHttpFetcher` / `FinraOtcSummaryHttpFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off the typed models, the pipe-row parsers
(`parse_short_interest_row`, `parse_otc_summary_row`, and their multi-line
variants), and the `MockShortInterestFetcher` / `MockOtcSummaryFetcher` helpers
are all available with no network dependencies.

## Environment variables

| Variable          | Required | Purpose |
| ----------------- | -------- | ------- |
| `TDW_FINRA_LIVE`  | no       | Set to `1` to enable the live network integration test. |

No API key is required.

## Quickstart

Offline (default features):

```rust
use tdw_provider_finra::parse_short_interest_response;

let body = "APPLE INC|AAPL|G|108234568|107982345|0.23|56789012|1.9|2024-01-15";
for rec in parse_short_interest_response(body)? {
    println!("{} {} short={}", rec.symbol, rec.settlement_date, rec.current_short_interest);
}
# Ok::<(), tdw_provider_finra::FinraProviderError>(())
```

Live HTTP (requires `--features http`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_finra::FinraShortInterestHttpFetcher;

let fetcher = FinraShortInterestHttpFetcher::default();
let rows = fetcher
    .fetch(serde_json::json!({ "limit": 25, "offset": 0 }), &Credentials::default())
    .await?;
```

## Example

```bash
cargo run -p tdw-provider-finra --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider feature-gating model and env-var conventions.
