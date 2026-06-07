# Architecture — tdw-provider-alpha-vantage

## Module map

| Module                | Feature | Responsibility                                                                       |
| --------------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`              | always  | Constants, `AlphaVantageFunction`, `AlphaVantageQuery` + validation, `AlphaVantageError`, `stub_*` helpers. |
| `http_fetcher.rs`     | `http`  | `AlphaVantageHttpFetcher`, private `Av*` wire structs, the `Fetcher` impl, transform helpers. |
| `tests/http_fetcher.rs` | `http` | Cassette tests for both functions, error-envelope tests, `TDW_ALPHA_VANTAGE_LIVE` live test. |

Constants in `lib.rs`: `PROVIDER_ID = "alpha_vantage"`, `BASE_URL`,
`API_KEY_ENV = "TDW_ALPHA_VANTAGE_API_KEY"`.

## Traits implemented

`AlphaVantageHttpFetcher` implements
`tdw_core::Fetcher<AlphaVantageQuery, MarketDataBar>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impl is
hand-written.

- `PROVIDER = "alpha_vantage"`, `ENDPOINT = "market_data"`.
- `registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`.
- `with_base_url(..)` redirects requests to a mock server for testing.

`AlphaVantageFunction` enumerates the supported functions and maps to the
`function=` API value via `as_api_str()`.

## Request → transform → data flow

1. **`transform_query(Value) -> AlphaVantageQuery`** — reads `symbol` (or the
   `ticker` alias) and optional `function` (default `TIME_SERIES_DAILY`; unknown
   functions are rejected). Delegates to `AlphaVantageQuery::new`, which
   upper-cases the symbol and rejects characters outside `[A-Za-z0-9._-]` (blocks
   `apikey=`/path injection).
2. **`extract_data(&query, &Credentials) -> Bytes`** — reads
   `TDW_ALPHA_VANTAGE_API_KEY`, builds a `reqwest` client, and GETs `BASE_URL`
   with `function`, `symbol`, `outputsize=compact`, and `apikey` query params.
   Non-2xx becomes `Error::Provider`.
3. **`transform_data(&query, Bytes) -> Vec<MarketDataBar>`** — dispatches on the
   query function:
   - `TimeSeriesDaily`: parses the `Time Series (Daily)` map into `MarketDataBar`
     rows (string OHLCV parsed to `f64`, `ts` = `"{date}T00:00:00Z"`), sorted
     ascending by `ts`. The resolved symbol comes from `Meta Data."2. Symbol"`.
   - `GlobalQuote`: parses the single `Global Quote` object into one snapshot
     `MarketDataBar` (price fills OHLC; empty `ts`).
   - Any `Information`/`Note` envelope (e.g. rate-limit) becomes
     `alpha_vantage api message: ...`; an empty `Global Quote` errors too.

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "alpha_vantage", "market_data")`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio`/`tdw-domain` and no `http_fetcher` unless
  `http` is enabled, so workspace builds and tests are offline.
- `stub_time_series_daily` / `stub_global_quote` return deterministic JSON for
  offline use.
- The **cassette tests** feed recorded Alpha Vantage JSON byte slices into
  `transform_data` and assert on the mapped rows (and on the rate-limit / empty
  error paths). [`examples/basic.rs`](examples/basic.rs) reproduces both happy
  paths offline.
- The **live test** requires `http`, `TDW_ALPHA_VANTAGE_LIVE=1`, and the API key.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against the documented `/query` endpoint.
- Wire structs (`Av*`) are private; only `MarketDataBar` crosses the boundary.
