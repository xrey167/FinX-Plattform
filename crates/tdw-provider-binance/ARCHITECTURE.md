# Architecture — tdw-provider-binance

## Module map

| Module                | Feature      | Responsibility                                                                   |
| --------------------- | ------------ | ------------------------------------------------------------------------------- |
| `lib.rs`              | always       | Constants, `BinanceTickerPriceQuery`/`BinanceTickerPrice`, `ProviderEndpoint`/`ProviderRequest`, `endpoints()`, `ticker_price_request`, `BinanceProviderError`. |
| `http_fetcher.rs`     | `http`       | `BinanceHttpTickerPriceFetcher`, private envelope structs, the `Fetcher` impl.   |
| `ws_streamer.rs`      | always (live socket under `ws`) | `BinanceTradeQuery`, `BinanceTradeStreamer`, pure `decode_trade_frame`, offline `snapshot`/`VecStream`. |
| `tests/http_fetcher.rs` | `http`     | Ticker cassette + error-envelope tests, `TDW_BINANCE_LIVE` live test.            |

Constants in `lib.rs`: `PROVIDER_ID = "binance"`, `BASE_URL`.

## Traits implemented

Both surfaces implement their `tdw_core` trait directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo.

| Type                            | Trait                                          | `PROVIDER` / `ENDPOINT`  |
| ------------------------------- | ---------------------------------------------- | ------------------------ |
| `BinanceHttpTickerPriceFetcher` | `Fetcher<BinanceTickerPriceQuery, BinanceTickerPrice>` | `binance` / `ticker_price` |
| `BinanceTradeStreamer`          | `Streamer<BinanceTradeQuery, Tick>`            | `binance` / `trades`     |

Each exposes a `registry_entry()` (`RegistryEntry::fetcher` / `::streamer`); the
fetcher also has `with_base_url(..)` for mock-server testing.

## Request → transform → data flow

### REST ticker (`Fetcher`)

1. **`transform_query`** — reads `symbol`, validates via
   `BinanceTickerPriceQuery::new` (upper-cased, ASCII-alphanumeric only — rejects
   `&recvWindow=` injection).
2. **`extract_data`** — re-checks the request contract via `ticker_price_request`,
   then GETs `/api/v3/ticker/price?symbol=...` (no credentials). Non-2xx →
   `Error::Provider`.
3. **`transform_data`** — first tries to parse a `{code,msg}` error envelope
   (surfaced as `binance api error ...`), else parses the `{symbol, price}`
   envelope and parses the string `price` into `f64`.

### Trade stream (`Streamer`)

- `decode_trade_frame(&str)` is a pure, IO-free decoder: it accepts the
  single-stream bare trade object and the combined-stream `{stream,data}` wrapper,
  maps `s`/`p`/`q`/`T` into a `Tick` (`venue = "BINANCE"`, `ts` = RFC3339 from
  epoch millis), and returns `Ok(vec![])` for non-trade frames (acks, ping
  results) and blank input.
- Under `ws`, `subscribe` opens the live socket and runs each text frame through
  `decode_trade_frame`. Without `ws`, `subscribe` returns a deterministic
  single-tick `VecStream`. `snapshot` is always deterministic and offline.

## Offline-default + cassette design

- `default = []`: no `reqwest` (`http`) and no live socket (`ws`) unless enabled,
  so workspace builds and tests are offline.
- The **ticker cassette tests** feed recorded JSON byte slices into
  `transform_data` and assert on the parsed row (plus the error-envelope path).
- The streamer's tests exercise `decode_trade_frame` directly across both frame
  shapes, non-trade events, blank input, and malformed price/JSON — making it the
  offline unit-test seam for the live subscribe path.
- [`examples/basic.rs`](examples/basic.rs) reproduces the ticker cassette and a
  `decode_trade_frame` call offline.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` and `tokio-tungstenite` against documented
  public endpoints.
- Envelope structs are private; only `BinanceTickerPrice`/`Tick` cross the
  boundary.
