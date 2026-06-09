# Architecture — tdw-provider-bls

## Module map

| Module                | Feature | Responsibility                                                                       |
| --------------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`              | always  | Constants/limits, `BlsSeriesQuery` + validation, `BlsDataPoint`, `BlsProviderError`, `BlsMockFetcher`, the shared `parse_bls_response`. |
| `http_fetcher.rs`     | `http`  | `BlsHttpTimeSeriesFetcher` and its `Fetcher` impl (delegates parsing to `parse_bls_response`). |

Constants in `lib.rs`: `PROVIDER_ID = "bls"`, `BASE_URL`,
`API_KEY_ENV = "TDW_BLS_API_KEY"`, plus `MAX_SERIES_IDS`, `MIN_YEAR`, `MAX_YEAR`.

## Traits implemented

`BlsHttpTimeSeriesFetcher` implements
`tdw_core::Fetcher<BlsSeriesQuery, BlsDataPoint>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impl is
hand-written.

- `PROVIDER = "bls"`, `ENDPOINT = "timeseries_data"`.
- `registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`.
- `with_base_url(..)` redirects requests to a mock server for testing.

The JSON parsing lives in the crate-internal `parse_bls_response` so it can be
exercised offline (via `BlsMockFetcher::parse_response`) and reused by the HTTP
fetcher — a single source of truth for decoding.

## Request → transform → data flow

1. **`transform_query(Value) -> BlsSeriesQuery`** — reads the `series_ids` array,
   `start_year`, and `end_year`, then delegates to `BlsSeriesQuery::new`, which
   enforces 1..=50 IDs (each alphanumeric/`_`/`-`), years in `[1900, 2100]`, and
   `start_year ≤ end_year`.
2. **`extract_data(&query, &Credentials) -> Bytes`** — reads the optional
   `TDW_BLS_API_KEY`, builds the POST JSON body
   (`seriesid`/`startyear`/`endyear`, plus `registrationkey` when a key is
   present), and POSTs to `/timeseries/data/`. Non-2xx becomes `Error::Provider`.
3. **`transform_data(&query, Bytes) -> Vec<BlsDataPoint>`** — parses the body into
   a `serde_json::Value`, surfaces any non-success `message` array as an error,
   then calls `parse_bls_response`, which checks `status == "REQUEST_SUCCEEDED"`,
   walks `Results.series[].data[]`, and parses each string `value` into `f64`.

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "bls", "timeseries_data")`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and no
  `http_fetcher` unless `http` is enabled, so workspace builds and tests are
  offline. (`serde_json` is non-optional because the shared parser lives in
  `lib.rs`.)
- `BlsMockFetcher::fetch_stub` returns one deterministic row per series ID, and
  `BlsMockFetcher::parse_response` exposes the real parser for **cassette tests**
  that feed recorded JSON without the `http` feature.
- The **live test** requires `http` and `TDW_BLS_LIVE=1`.
- [`examples/basic.rs`](examples/basic.rs) reproduces the cassette path offline.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against the documented public API.
- A single parsing path (`parse_bls_response`) is shared by the mock and HTTP
  fetcher, so offline tests validate the exact production decode.
