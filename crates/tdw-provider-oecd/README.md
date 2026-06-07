# tdw-provider-oecd

Market-data provider for the **OECD Statistics** SDMX-JSON API. Exposes a typed
query/observation model and a `tdw_core::Fetcher` implementation for arbitrary
dataset slices.

- **Vendor:** OECD — SDMX-JSON Statistics API
- **Base URL:** `https://stats.oecd.org/SDMX-JSON/data`
- **Endpoint:** `sdmx_data` — `GET /{dataset}/{filter}/OECD?startTime=…&endTime=…`
- **Auth:** none (public API)

## Feature flags

| Feature | Default | Effect |
| ------- | ------- | ------ |
| `http`  | off     | Compiles `OecdHttpDataFetcher` and pulls in `reqwest`, `tokio`, `async-trait`, `bytes`, `tdw-core`, `tdw-domain`. |

With `http` off the typed models, dataset validation, the `sdmx_data_url`
builder, the `endpoints()` metadata, and the `MockOecdFetcher` are available
with no network dependencies.

## Environment variables

| Variable          | Required | Purpose |
| ----------------- | -------- | ------- |
| `TDW_OECD_LIVE`   | no       | Set to `1` to enable the live network integration test. |

No API key is required.

## Quickstart

Offline (default features):

```rust
use tdw_provider_oecd::{MockOecdFetcher, OecdQuery};

let query = OecdQuery::new("QNA", "AUS.B1_GE.Q", "2020", "2023")?;
for row in MockOecdFetcher.fetch(&query)? {
    println!("{} {} = {}", row.dataset, row.period, row.value);
}
# Ok::<(), tdw_provider_oecd::OecdProviderError>(())
```

Live HTTP (requires `--features http`):

```rust,ignore
use tdw_core::{Credentials, Fetcher};
use tdw_provider_oecd::OecdHttpDataFetcher;

let fetcher = OecdHttpDataFetcher::default();
let rows = fetcher
    .fetch(serde_json::json!({
        "dataset": "QNA", "filter": "AUS.B1_GE.Q",
        "start_time": "2020", "end_time": "2023"
    }), &Credentials::default())
    .await?;
```

The `filter`, `start_time`, and `end_time` fields use OECD SDMX dimension-key
notation and are passed through verbatim; only `dataset` is character-validated.

## Example

```bash
cargo run -p tdw-provider-oecd --example basic --features http
```

## Configuration

See [`docs/CONFIGURATION.md`](../../docs/CONFIGURATION.md) for the workspace-wide
provider feature-gating model and env-var conventions.
