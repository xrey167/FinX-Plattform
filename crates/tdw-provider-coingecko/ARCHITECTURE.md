# Architecture — tdw-provider-coingecko

## Module map

| Module            | Feature | Responsibility                                                                       |
| ----------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`          | always  | Constants, `CoinGeckoOhlcQuery` + validation, `ProviderRequest`, `ohlc_request`, `CoinGeckoProviderError`. |
| `http_fetcher.rs` | `http`  | `CoinGeckoHttpOhlcFetcher`, the `Fetcher` impl, internal millis→ISO timestamp helper, inline cassette tests. |

Constants in `lib.rs`: `PROVIDER_ID = "coingecko"`, `BASE_URL`,
`API_KEY_HEADER = "x-cg-demo-api-key"`. The optional API-key env var
(`COINGECKO_API_KEY`) is read inside `http_fetcher`.

## Traits implemented

`CoinGeckoHttpOhlcFetcher` implements
`tdw_core::Fetcher<CoinGeckoOhlcQuery, MarketDataBar>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impl is
hand-written.

- `PROVIDER = "coingecko"`, `ENDPOINT = "ohlc"`.
- `registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`.
- `with_base_url(..)` redirects requests to a mock server for testing.

## Request → transform → data flow

1. **`transform_query(Value) -> CoinGeckoOhlcQuery`** — reads `coin_id` (alias
   `symbol`), `vs_currency` (default `usd`), and `days` (default `30`). Delegates
   to `CoinGeckoOhlcQuery::new`, which lower-cases the coin id (alphanumeric/`-`/`_`)
   and currency (alphanumeric) and restricts `days` to the documented set
   (1/7/14/30/90/180/365).
2. **`extract_data(&query, &Credentials) -> Bytes`** — re-checks the contract via
   `ohlc_request`, builds the `/coins/{id}/ohlc?vs_currency=&days=` URL, and
   forwards the optional `COINGECKO_API_KEY` as the `x-cg-demo-api-key` header
   when present. Non-2xx becomes `Error::Provider`.
3. **`transform_data(&query, Bytes) -> Vec<MarketDataBar>`** — CoinGecko's OHLC
   response is a JSON array of `[ts_ms, open, high, low, close]` arrays; each is
   mapped to a `MarketDataBar` (`venue`/`source` = `coingecko`,
   `TimeGranularity::Day`, `volume = 0.0` since OHLC carries no volume, `ts` via
   the dependency-free `unix_millis_to_iso_timestamp`).

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "coingecko", "ohlc")`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and no
  `http_fetcher` unless `http` is enabled, so workspace builds and tests are
  offline.
- `ohlc_request` is unit-tested for path construction and validation; the
  timestamp helper is tested against well-known dates.
- Under `http`, the **inline cassette test** feeds a recorded OHLC array into
  `transform_data` and asserts on the mapped bar — exactly what
  [`examples/basic.rs`](examples/basic.rs) reproduces.
- The **live test** is gated by `http` and `TDW_COINGECKO_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against the documented v3 endpoint.
- Timestamp conversion is implemented in-crate to avoid a `chrono` dependency.
