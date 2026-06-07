# Architecture — tdw-provider-ccdata

## Module map

| Module            | Feature | Responsibility                                                                       |
| ----------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`          | always  | Constants, `CCDataOhlcvQuery`/`CCDataAssetQuery` + validation, `CCDataError`, `stub_*_response`. |
| `http_fetcher.rs` | `http`  | `CCDataHttpFetcher` (OHLCV) + `CCDataAssetHttpFetcher` (asset), private `Cc*` wire structs, `CCDataAssetRow`, the two `Fetcher` impls, internal date helpers. |

Constants in `lib.rs`: `PROVIDER_ID = "ccdata"`, `BASE_URL`,
`API_KEY_ENV = "TDW_CCDATA_API_KEY"`.

> Note: only `CCDataHttpFetcher` is re-exported at the crate root;
> `CCDataAssetHttpFetcher` and `CCDataAssetRow` are reached via the
> `tdw_provider_ccdata::http_fetcher` module path.

## Traits implemented

Each fetcher implements `tdw_core::Fetcher<Q, D>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impls are
hand-written.

| Fetcher                  | `Q`               | `D`             | `PROVIDER` / `ENDPOINT`   |
| ------------------------ | ----------------- | --------------- | ------------------------- |
| `CCDataHttpFetcher`      | `CCDataOhlcvQuery`| `MarketDataBar` | `ccdata` / `crypto_ohlcv` |
| `CCDataAssetHttpFetcher` | `CCDataAssetQuery`| `CCDataAssetRow`| `ccdata` / `crypto_asset` |

`CCDataHttpFetcher` exposes `registry_entry()`; both provide `with_base_url(..)`.

## Request → transform → data flow

1. **`transform_query(Value) -> Q`** — OHLCV reads `market` (default `ccix`),
   `instrument`, and `limit` (default `30`); asset reads `symbol` (alias
   `asset_symbol`). Both delegate to the query constructors: instruments must
   match `ASSET-CURRENCY` (exactly one `-`, alphanumeric parts, upper-cased);
   asset symbols are alphanumeric, upper-cased.
2. **`extract_data(&Q, &Credentials) -> Bytes`** — reads `TDW_CCDATA_API_KEY`,
   builds a `reqwest` client, and GETs the endpoint with the
   `authorization: Apikey <key>` header and query params. Non-2xx becomes
   `Error::Provider`.
3. **`transform_data(&Q, Bytes) -> Vec<D>`** — parses the `{ "Data": { ... } }`
   envelope into private wire structs:
   - OHLCV: each `Entries[]` becomes a `MarketDataBar` (`venue`/`source` =
     `ccdata`, `TimeGranularity::Day`, `ts` from the `TIMESTAMP` seconds via the
     dependency-free `format_unix_ts`), sorted ascending by `ts`.
   - Asset: the single `Data` object becomes one `CCDataAssetRow`.

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "ccdata", ENDPOINT)`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and no
  `http_fetcher` unless `http` is enabled, so workspace builds and tests are
  offline. (`serde_json` is non-optional.)
- `stub_ohlcv_response` / `stub_asset_response` return deterministic JSON for
  offline use; `format_unix_ts` is unit-tested against known epochs.
- Under `http`, **cassette tests** feed recorded JSON byte slices into
  `transform_data` and assert on the parsed rows — exactly what
  [`examples/basic.rs`](examples/basic.rs) reproduces.
- The **live test** requires `http`, `TDW_CCDATA_LIVE=1`, and the API key.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against documented REST paths.
- Timestamp conversion is implemented in-crate (Fliegel & Van Flandern) to avoid
  a `chrono` dependency.
- Wire structs are private; only `MarketDataBar`/`CCDataAssetRow` cross the
  boundary.
