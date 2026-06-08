# tdw-provider-tradier — Architecture

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query types (`TradierQuoteQuery`, `TradierOptionsQuery`), data models (`TradierQuote`, `TradierOptionContract`), error enum (`TradierProviderError`), validation, and the `PROVIDER_ID` / `BASE_URL` / `API_KEY_ENV` constants. |
| `http_fetcher.rs` | `feature = "http"` | The two `Fetcher` implementations, private wire structs, and the shared `read_api_key` / `build_client` helpers. |

## Traits implemented

Both fetchers implement `tdw_core::Fetcher<Q, D>`:

| Type | `Q` | `D` | `PROVIDER` / `ENDPOINT` |
| ---- | --- | --- | ----------------------- |
| `TradierHttpQuoteFetcher` | `TradierQuoteQuery` | `Quote` | `tradier` / `quote` |
| `TradierHttpOptionsFetcher` | `TradierOptionsQuery` | `EquityHistoricalData` | `tradier` / `options_chain` |

`Quote` and `EquityHistoricalData` are shared `tdw_domain` models.

## Data flow

```
transform_query (Value -> Q)  ->  extract_data (Q -> Bytes, async IO)
                              ->  transform_data (Bytes -> Vec<D>, pure)
```

1. `transform_query` reads `symbol` (quotes) or `symbol` + `expiration`
   (options) from the JSON `Value` and validates via the `lib.rs`
   constructors.
2. `extract_data` reads `TDW_TRADIER_API_KEY`, sets
   `Authorization: Bearer …` and `Accept: application/json`, and issues the
   GET. Non-2xx becomes `Error::Provider`.
3. `transform_data` deserialises the Tradier envelopes
   (`quotes.quote.*`, `options.option[].*`) into the public domain models.

## Offline / cassette design

`transform_data` is pure over `Bytes`, so the full parsing path is tested and
demonstrated offline against inline JSON cassettes that mirror Tradier's
response shapes. `with_base_url(..)` retargets `extract_data` at a local stub
in integration tests. Live network access requires the `http` feature and a
real token.

## Clean-room invariants

- `#![forbid(unsafe_code)]` via workspace lints.
- No captured Tradier responses are committed; only synthetic fixtures shaped
  like the documented envelopes appear in tests and the example.
- `reqwest` / `tokio` are optional, gated behind `http`; the default build is
  offline and deterministic.
- The crate talks only to documented Tradier REST endpoints over the bearer
  token — no scraping or private APIs.
