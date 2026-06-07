# Architecture — tdw-provider-alpaca

## Module map

| Module                | Feature | Responsibility                                                                          |
| --------------------- | ------- | -------------------------------------------------------------------------------------- |
| `lib.rs`              | always  | Constants/headers, `AlpacaStockBarsQuery` (+ builders), `ProviderEndpoint`/`ProviderRequest`, `endpoints()`, `stock_bars_request`, `AlpacaProviderError`. |
| `http_fetcher.rs`     | `http`  | `AlpacaHttpStockBarsFetcher`, private `AlpacaEnvelope`/`AlpacaBar` wire structs, the `Fetcher` impl. |
| `tests/http_fetcher.rs` | `http` | Cassette test (bars + error-envelope), symbol-normalisation test, `TDW_ALPACA_LIVE` live test. |

Constants in `lib.rs`: `PROVIDER_ID = "alpaca"`, `BASE_URL`,
`API_KEY_HEADER = "APCA-API-KEY-ID"`, `API_SECRET_HEADER = "APCA-API-SECRET-KEY"`.

## Traits implemented

`AlpacaHttpStockBarsFetcher` implements
`tdw_core::Fetcher<AlpacaStockBarsQuery, MarketDataBar>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impl is
hand-written.

- `PROVIDER = "alpaca"`, `ENDPOINT = "stock_bars"`.
- `registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`.
- `with_base_url(..)` redirects requests to a mock server for testing.

The offline `lib.rs` also models the request contract without secrets:
`stock_bars_request` returns a `ProviderRequest` (path + header names) and errors
with `MissingApiKey` when no key is signalled — used to unit-test the contract
without touching the network.

## Request → transform → data flow

1. **`transform_query(Value) -> AlpacaStockBarsQuery`** — reads `symbol`, `start`,
   `end` (required), plus optional `timeframe` (default `1Day`), `limit` (default
   `1000`), and `feed`. Delegates to `AlpacaStockBarsQuery::new` + the `with_*`
   builders, which upper-case the symbol, reject path/query-injection characters,
   require `YYYY-MM-DD` dates, and reject zero limits.
2. **`extract_data(&query, &Credentials) -> Bytes`** — validates the request
   contract, reads `APCA_API_KEY_ID`/`APCA_API_SECRET_KEY` from the environment,
   builds a `reqwest` client, and GETs `/v2/stocks/bars` with the auth headers and
   query parameters. Non-2xx becomes `Error::Provider`.
3. **`transform_data(&query, Bytes) -> Vec<MarketDataBar>`** — deserialises the
   `AlpacaEnvelope` (`bars` keyed by symbol; optional `code`/`message`). An error
   envelope (no bars + code/message) surfaces as `alpaca api error ...`. Otherwise
   the bars for the query symbol are mapped into `MarketDataBar` rows
   (`venue`/`source` = `alpaca`, `TimeGranularity::Day`, `ts` = Alpaca's `t`).

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "alpaca", "stock_bars")`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio`/`tdw-domain` and no `http_fetcher` unless
  `http` is enabled, so workspace builds and tests are offline.
- The **cassette tests** feed recorded Alpaca JSON byte slices into
  `transform_data` and assert on the mapped rows — and separately assert the
  error-envelope path. [`examples/basic.rs`](examples/basic.rs) reproduces the
  happy path offline.
- The **live test** requires `http`, `TDW_ALPACA_LIVE=1`, and the two API keys.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against the documented REST path.
- Secret material is only ever read inside `extract_data`; the offline contract
  (`ProviderRequest`) never carries credentials.
- Wire structs are private; only `MarketDataBar` crosses the boundary.
