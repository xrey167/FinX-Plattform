# tdw-provider-bls

Bureau of Labor Statistics (BLS) economic-time-series provider for the TDW
platform. Wraps the BLS public v2 API (`https://api.bls.gov/publicAPI/v2`) and
returns `BlsDataPoint` rows via a single `tdw_core::Fetcher`.

| Endpoint            | Fetcher                       | Path                                | Row model       |
| ------------------- | ----------------------------- | ----------------------------------- | --------------- |
| Time-series data    | `BlsHttpTimeSeriesFetcher`    | `POST /timeseries/data/`            | `BlsDataPoint`  |

A BLS API key is **optional** — without one the public rate limits apply; with
one (registration key) you get higher throughput. The crate compiles and tests
offline by default: `BlsSeriesQuery` validation, the `BlsMockFetcher` (stub +
`parse_response`), and the shared parser are always available, while the
network-backed fetcher and `reqwest` dependency only exist under the `http`
feature.

## Feature flags

| Feature  | Default | Effect                                                                                          |
| -------- | ------- | ----------------------------------------------------------------------------------------------- |
| `http`   | off     | Pulls in `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and compiles the live `http_fetcher` module. |

```bash
cargo build -p tdw-provider-bls --features http
```

## Environment variables

| Variable           | Required for           | Purpose                                                              |
| ------------------ | ---------------------- | ------------------------------------------------------------------- |
| `TDW_BLS_API_KEY`  | optional               | Sent as the `registrationkey` body field for higher rate limits.    |
| `TDW_BLS_LIVE=1`   | live integration test  | Opt-in gate; without it the live test skips so CI stays offline.    |

## Quickstart

```rust
use tdw_core::{Credentials, Fetcher};
use tdw_provider_bls::BlsHttpTimeSeriesFetcher;

# async fn run() -> tdw_core::Result<()> {
let fetcher = BlsHttpTimeSeriesFetcher::default();
let obb = fetcher
    .fetch(
        serde_json::json!({
            "series_ids": ["CUUR0000SA0"],
            "start_year": 2023,
            "end_year": 2024
        }),
        &Credentials::default(),
    )
    .await?;
for point in obb.rows {
    println!("{} {}/{} = {}", point.series_id, point.year, point.period, point.value);
}
# Ok(())
# }
```

`series_ids` (1..=50 IDs, alphanumeric/`_`/`-`) plus `start_year`/`end_year`
(1900..=2100, start ≤ end) are required.

For an offline run that mirrors the cassette tests, see
[`examples/basic.rs`](examples/basic.rs):

```bash
cargo run -p tdw-provider-bls --features http --example basic
```

## Further reading

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — module map, data flow, clean-room design.
- Platform docs index: [`docs/index.md`](../../docs/index.md).
