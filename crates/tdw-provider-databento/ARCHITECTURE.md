# Architecture — tdw-provider-databento

## Module map

| Module            | Feature | Responsibility                                                                       |
| ----------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`          | always  | Constants, `DatabentoHistoricalQuery` / `DatabentoTimeseriesQuery` + validation, `DatabentoError`. |
| `http_fetcher.rs` | `http`  | `DatabentoHttpTimeseriesFetcher` + `DatabentoMetadataFetcher`, private response structs, `DatabentoMetadataQuery`/`DatabentoDataset`, the two `Fetcher` impls, nanos→ISO timestamp helper. |

Constants in `lib.rs`: `PROVIDER_ID = "databento"`, `BASE_URL`,
`API_KEY_ENV = "TDW_DATABENTO_API_KEY"`.

## Traits implemented

Each fetcher implements `tdw_core::Fetcher<Q, D>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impls are
hand-written.

| Fetcher                          | `Q`                        | `D`                | `PROVIDER` / `ENDPOINT`   |
| -------------------------------- | -------------------------- | ------------------ | ------------------------- |
| `DatabentoHttpTimeseriesFetcher` | `DatabentoTimeseriesQuery` | `MarketDataBar`    | `databento` / `timeseries`|
| `DatabentoMetadataFetcher`       | `DatabentoMetadataQuery`   | `DatabentoDataset` | `databento` / `metadata`  |

Both expose `registry_entry()` and `with_base_url(..)`. `DatabentoHistoricalQuery`
in `lib.rs` is a single-symbol convenience wrapper; the timeseries fetcher uses
`DatabentoTimeseriesQuery` (dataset + symbol list + date range).

## Request → transform → data flow

1. **`transform_query(Value) -> Q`** — timeseries reads `dataset`, `symbols`
   (array), `start`, `end` and delegates to `DatabentoTimeseriesQuery::new`
   (non-empty dataset/symbols, `YYYY-MM-DD` dates). Metadata takes no params and
   returns the default query.
2. **`extract_data(&Q, &Credentials) -> Bytes`** — reads
   `TDW_DATABENTO_API_KEY` and authenticates with HTTP Basic
   (`basic_auth(key, Some(""))`). Timeseries POSTs a JSON body
   (`dataset`/`symbols`/`schema: "ohlcv-1d"`/dates/`encoding: "json"`) to
   `/timeseries.get_range`; metadata GETs `/metadata.list_datasets`. Non-2xx
   becomes `Error::Provider`.
3. **`transform_data(&Q, Bytes) -> Vec<D>`** —
   - timeseries: parses `{ "records": [...] }`, mapping each OHLCV record to a
     `MarketDataBar` (`symbol` = the single symbol or comma-joined list, `venue`
     = dataset, `source` = `databento`, `TimeGranularity::Day`, `ts` from the
     `ts_event` nanos via `unix_nanos_to_iso_timestamp`).
   - metadata: parses `{ "result": [...] }` into `DatabentoDataset { id }` rows.

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "databento", ENDPOINT)`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and no
  `http_fetcher` unless `http` is enabled, so workspace builds and tests are
  offline.
- Query validation is unit-tested in `lib.rs`; the timestamp helper is tested
  against known epochs.
- Under `http`, **cassette tests** feed recorded JSON byte slices into
  `transform_data` and assert on the parsed rows — exactly what
  [`examples/basic.rs`](examples/basic.rs) reproduces.
- The **live test** requires `http`, `TDW_DATABENTO_LIVE=1`, and the API key.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against documented REST paths with Basic auth.
- Timestamp conversion is implemented in-crate to avoid a `chrono` dependency.
- Response structs are private; only `MarketDataBar`/`DatabentoDataset` cross the
  boundary.
