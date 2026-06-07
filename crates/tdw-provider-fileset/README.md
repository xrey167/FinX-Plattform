# tdw-provider-fileset

A **fixture-backed** market-data provider. It implements the
`tdw_core::Fetcher` contract for equity historical bars but serves
deterministic in-memory rows instead of calling any external vendor. It exists
so the registry, dispatch, and downstream pipelines can be exercised
end-to-end with zero network and zero credentials.

- **Vendor:** none — synthetic / fixture data
- **Endpoint:** `equity_historical`
- **Auth:** none
- **Output model:** `tdw_domain::EquityHistoricalData`

## Feature flags

This crate has **no feature flags**. Unlike the network providers it has no
`http` feature: `async-trait`, `bytes`, `tdw-core`, and `tdw-domain` are
unconditional dependencies because the `Fetcher` impl is always compiled. There
is no `reqwest` dependency and no network code at all.

## Environment variables

None. The fetcher is fully offline and requires no configuration.

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_fileset::FilesetEquityHistoricalFetcher;

# async fn run() -> tdw_core::Result<()> {
let fetcher = FilesetEquityHistoricalFetcher;
let obb = fetcher
    .fetch(serde_json::json!({ "symbol": "aapl" }), &Credentials::default())
    .await?;
for row in obb.results {
    println!("{} {} close={}", row.symbol, row.date, row.close);
}
# Ok(())
# }
```

`transform_query` normalises and validates the symbol (uppercased; only ASCII
alphanumeric plus `.`, `-`, `_`), so path-traversal-style inputs are rejected.
The `fixture_rows(symbol)` helper exposes the same canned rows directly.

## Example

```bash
cargo run -p tdw-provider-fileset --example basic
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace
provider conventions. This crate intentionally needs none of them.
