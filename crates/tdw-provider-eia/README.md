# tdw-provider-eia

Market-data provider for the **U.S. Energy Information Administration (EIA)
API v2**. Exposes typed query/record models plus two `tdw_core::Fetcher`
implementations for petroleum spot prices and natural-gas prices.

- **Vendor:** U.S. EIA — Open Data API v2
- **Base URL:** `https://api.eia.gov/v2`
- **Endpoints:**
  - `spot_price` — `GET /petroleum/pri/spt/data/` (daily)
  - `natural_gas` — `GET /natural-gas/pri/sum/data/` (monthly)
- **Auth:** API key passed as the `api_key` query parameter.

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `EiaHttpSpotPriceFetcher` / `EiaHttpNaturalGasFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off the typed models, validation, and the `mock_spot_price` /
`mock_natural_gas` stubs are available with no network dependencies.

## Environment variables

| Variable             | Required | Purpose |
| -------------------- | -------- | ------- |
| `TDW_EIA_API_KEY`    | for live calls | EIA API key, appended as `api_key`. |
| `TDW_EIA_LIVE`       | no       | Set to `1` to enable the live network integration test. |

The provider-id and env-var name are exported as `PROVIDER_ID`, `API_KEY_ENV`.

## Quickstart

Offline (default features):

```rust
use tdw_provider_eia::{EiaCommodity, EiaSpotPriceQuery, mock_spot_price};

let query = EiaSpotPriceQuery::new(EiaCommodity::CrudeOilWti, 10)?;
for row in mock_spot_price(&query)? {
    println!("{} {} = {} {}", row.period, row.product_name, row.value, row.units);
}
# Ok::<(), tdw_provider_eia::EiaProviderError>(())
```

Live HTTP (requires `--features http` and `TDW_EIA_API_KEY`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_eia::EiaHttpSpotPriceFetcher;

let fetcher = EiaHttpSpotPriceFetcher::default();
let query = EiaHttpSpotPriceFetcher::transform_query(serde_json::json!({
    "commodity": "crude_oil_wti", "length": 10,
}))?;
let rows = fetcher.fetch(serde_json::to_value(&query)?, &Credentials::default()).await?;
```

## Example

```bash
cargo run -p tdw-provider-eia --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider env-var conventions and feature-gating model.
