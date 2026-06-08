# tdw-provider-tiingo — Architecture

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query types (`TiingoHistoricalQuery`, `TiingoNewsQuery`), error enum (`TiingoProviderError`), validation, and the `PROVIDER_ID` / `BASE_URL` / `API_KEY_ENV` constants. Pure, offline. |
| `http_fetcher.rs` | `feature = "http"` | The two `Fetcher` implementations, their wire structs, the public `TiingoNewsArticle` output type, and the shared `read_api_key` / `build_client` helpers. |

## Traits implemented

Both fetchers implement `tdw_core::Fetcher<Q, D>`:

| Type | `Q` | `D` | `PROVIDER` / `ENDPOINT` |
| ---- | --- | --- | ----------------------- |
| `TiingoHttpHistoricalFetcher` | `TiingoHistoricalQuery` | `MarketDataBar` | `tiingo` / `historical` |
| `TiingoHttpNewsFetcher` | `TiingoNewsQuery` | `TiingoNewsArticle` | `tiingo` / `news` |

The `Fetcher` default `fetch()` chains the three stages:

```
transform_query (Value -> Q)  ->  extract_data (Q -> Bytes, async IO)
                              ->  transform_data (Bytes -> Vec<D>, pure)
```

## Data flow

1. `transform_query` accepts a JSON `Value`. Historical reads `symbol` (or
   `ticker`); news reads `tickers` as a string or array. Both validate via
   the `lib.rs` query constructors.
2. `extract_data` reads `TDW_TIINGO_API_KEY`, builds the URL, and sends a
   `reqwest` GET with the `token` query parameter. Non-2xx responses become
   `Error::Provider`.
3. `transform_data` deserialises into private wire structs and maps to public
   rows: historical → `MarketDataBar` (`venue`/`source = "tiingo"`,
   daily granularity); news → `TiingoNewsArticle`.

## Offline / cassette design

`transform_data` is pure and takes `Bytes`, so the parsing path is tested and
demonstrated entirely offline with inline JSON cassettes that mirror Tiingo's
response shapes. `with_base_url(..)` retargets `extract_data` at a local stub
in integration tests. Live network access requires both the `http` feature and
`TDW_TIINGO_LIVE=1` (plus a real token).

## Clean-room invariants

- `#![forbid(unsafe_code)]` is enforced workspace-wide via lints.
- No captured Tiingo responses are committed; only synthetic fixtures shaped
  like the documented schema appear in tests and the example.
- `reqwest` / `tokio` are optional, gated behind `http`; the default build is
  offline and deterministic.
- The crate talks only to documented Tiingo REST endpoints over the token
  query parameter — no scraping or private APIs.
