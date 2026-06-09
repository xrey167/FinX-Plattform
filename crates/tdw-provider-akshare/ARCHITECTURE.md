# Architecture — tdw-provider-akshare

## Module map

| Module                | Feature | Responsibility                                                                  |
| --------------------- | ------- | ------------------------------------------------------------------------------- |
| `lib.rs`              | always  | `AkShareMarket`, `AkShareQuery` + validation, `AkShareError`, `fetch_stub`/`StubBar`. |
| `http_fetcher.rs`     | `http`  | `AkShareHttpFetcher`, private `AkShareBar` wire struct, the `Fetcher` impl.      |
| `tests/http_fetcher.rs` | `http` | Cassette tests for A-share + HK bars, parse-error test, `TDW_AKSHARE_LIVE` live test. |

Constants in `lib.rs`: `PROVIDER_ID = "akshare"`, `BASE_URL`.

## Traits implemented

`AkShareHttpFetcher` implements `tdw_core::Fetcher<AkShareQuery, MarketDataBar>`
directly — there is no `provider_fetcher_struct!` macro or `ProviderSpec` in this
repo; the impl is hand-written.

- `PROVIDER = "akshare"`, `ENDPOINT = "hist"`.
- `registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`.
- `with_base_url(..)` redirects requests to a local mirror for testing.

`AkShareMarket` encapsulates per-market behaviour: `endpoint_path()`, `venue()`,
and `expected_symbol_len()`.

## Request → transform → data flow

1. **`transform_query(Value) -> AkShareQuery`** — pulls `symbol`, optional
   `market` (defaults to A-shares; accepts the `hk`/`HK`/`HongKong` aliases),
   `start_date`, `end_date`, then delegates to `AkShareQuery::new`, which enforces
   ASCII-digit symbols of the market's expected length and `YYYYMMDD` dates.
2. **`extract_data(&AkShareQuery, &Credentials) -> Bytes`** — POSTs a JSON body
   (`symbol`/`period: "daily"`/dates/`adjust: ""`) to the market's endpoint path
   on the base URL. No credentials are read. Non-2xx becomes `Error::Provider`.
3. **`transform_data(&AkShareQuery, Bytes) -> Vec<MarketDataBar>`** — deserialises
   the JSON array into private `AkShareBar` structs (Chinese field names mapped via
   `#[serde(rename)]`), then builds `MarketDataBar` rows: the query symbol, the
   market venue, `TimeGranularity::Day`, and a `ts` of `"{date}T00:00:00Z"`.

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "akshare", "hist")`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio`/`tdw-domain` and no `http_fetcher` unless
  `http` is enabled, so workspace builds and tests are offline.
- `fetch_stub` returns a deterministic two-bar `StubBar` vector for offline use.
- The **cassette tests** feed recorded JSON byte slices (with the original
  Chinese field names) into `transform_data` and assert on the mapped rows —
  exactly what [`examples/basic.rs`](examples/basic.rs) reproduces. A bad-JSON
  test asserts the `akshare parse_json` error path.
- The **live test** requires both `http` and `TDW_AKSHARE_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against the documented bridge paths.
- Wire structs (`AkShareBar`) are private; only `MarketDataBar` crosses the
  boundary.
