# tdw-provider-tmx — Architecture

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query types (`TmxQuoteQuery`, `TmxBatchQuoteQuery`), the `TmxQuote` data model, the `TmxError` enum, symbol validation, the offline `TmxMockQuoteFetcher`, the pure `parse_quote_response` decoder, and `BATCH_MAX_SYMBOLS`. |
| `http_fetcher.rs` | `feature = "http"` | The two real `Fetcher` implementations and their HTTP plumbing. |

## Traits implemented

The real fetchers implement `tdw_core::Fetcher<Q, D>`:

| Type | `Q` | `D` | `PROVIDER` / `ENDPOINT` |
| ---- | --- | --- | ----------------------- |
| `TmxHttpQuoteFetcher` | `TmxQuoteQuery` | `EquityHistoricalData` | `tmx` / `equity_quote` |
| `TmxHttpBatchQuoteFetcher` | `TmxBatchQuoteQuery` | `EquityHistoricalData` | `tmx` / `equity_batch_quote` |

`TmxMockQuoteFetcher` is **not** a `Fetcher`; it is a plain struct exposing the
synchronous `fetch_mock(&Value) -> Result<Vec<TmxQuote>>` associated function,
so it can be used in any (sync or async) context with no runtime.

## Data flow

```
Real path:  transform_query -> extract_data (async HTTP) -> transform_data
Mock path:  fetch_mock (validate -> build TmxQuote -> return)  [sync, offline]
Pure parse: parse_quote_response (bytes -> Vec<TmxQuote>)      [offline]
```

1. Queries are validated up front (`from_params`); symbols are upper-cased and
   restricted to ASCII alphanumerics plus `.`/`-`/`_`. Batch queries reject
   more than `BATCH_MAX_SYMBOLS` entries.
2. The real fetchers call the TMX Money `getquote` endpoint and map the
   `{ "results": [...] }` envelope into `EquityHistoricalData`.
3. `parse_quote_response` decodes the same envelope into `TmxQuote` rows using
   `camelCase` field renaming; it performs no IO.

## Offline / mock design

Two offline seams keep the crate testable and demonstrable without a network:

- `TmxMockQuoteFetcher::fetch_mock` returns one deterministic hardcoded
  `TmxQuote` for the requested symbol — ideal for unit tests and the example.
- `parse_quote_response` is pure over `&[u8]`, so the decode path is exercised
  against inline JSON cassettes.

Both are compiled regardless of the `http` feature, so `examples/basic.rs`
runs with the default (offline) build.

## Clean-room invariants

- `#![forbid(unsafe_code)]` via workspace lints.
- No captured TMX responses are committed; only synthetic fixtures shaped like
  the documented `getquote` envelope appear in tests and the example.
- `reqwest` / `tokio` are optional, gated behind `http`; the default build is
  offline and deterministic.
- The crate talks only to the documented public TMX Money quote endpoint.
