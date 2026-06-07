# tdw-provider-ecb

Market-data provider for the **European Central Bank (ECB) Statistical Data
Warehouse** (SDW). It exposes typed query/observation models and a
`tdw_core::Fetcher` implementation for the SDW `/data/{flow}/{key}` endpoint.

- **Vendor:** European Central Bank — SDW REST API
- **Base URL:** `https://data-api.ecb.europa.eu/service`
- **Endpoint:** `data` — `GET /data/{flow}/{key}?format=jsondata`
- **Auth:** none (the ECB SDW is a public API)

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `EcbHttpDataFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` **off** the crate is pure-offline: the typed models, validation,
`data_request_path`, `parse_ecb_json`, and `EcbMockFetcher` are always
available with no network dependencies.

## Environment variables

| Variable        | Required | Purpose |
| --------------- | -------- | ------- |
| `TDW_ECB_LIVE`  | no       | Set to `1` to enable the live network integration test. Unset/`0` keeps it skipped so CI stays offline. |

No API key is required.

## Quickstart

Offline (default features):

```rust
use tdw_provider_ecb::{EcbDataQuery, EcbMockFetcher};

let query = EcbDataQuery::new("EXR", "D.USD.EUR.SP00.A", "2024-01-01", "2024-01-31")?;
let raw = br#"{ "dataSets": [ /* ... */ ] }"#.to_vec();
let rows = EcbMockFetcher { raw }.parse(&query)?;
for row in rows {
    println!("{} {} = {}", row.date, row.key, row.value);
}
# Ok::<(), tdw_provider_ecb::EcbProviderError>(())
```

Live HTTP (requires `--features http`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_ecb::EcbHttpDataFetcher;

let fetcher = EcbHttpDataFetcher::default();
let query = EcbHttpDataFetcher::transform_query(serde_json::json!({
    "flow": "EXR", "key": "D.USD.EUR.SP00.A",
    "start_period": "2024-01-01", "end_period": "2024-01-31",
}))?;
let rows = fetcher.fetch(serde_json::to_value(&query)?, &Credentials::default()).await?;
```

## Example

Run the compiled offline example (mirrors the cassette test fixture):

```bash
cargo run -p tdw-provider-ecb --example basic --features http
```

## Configuration

For environment-variable conventions and the provider feature-gating model
across the workspace, see [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md).
