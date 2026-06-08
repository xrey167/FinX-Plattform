# tdw-provider-yahoo — Architecture

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | The offline `YahooEquityHistoricalFetcher` — a complete `Fetcher` whose `extract_data` returns a deterministic synthetic bar. Query validation is delegated to `tdw-provider-fileset`. |
| `http_fetcher.rs` | `feature = "http"` | The real `YahooHttpEquityHistoricalFetcher`, the Yahoo chart-envelope wire structs, and the `unix_to_iso_date` date helper. |

## Traits implemented

Both fetchers implement `tdw_core::Fetcher<EquityHistoricalQuery,
EquityHistoricalData>` with the same registry identity:

| Type | Gate | `PROVIDER` / `ENDPOINT` |
| ---- | ---- | ----------------------- |
| `YahooEquityHistoricalFetcher` | always | `yahoo` / `equity_historical` |
| `YahooHttpEquityHistoricalFetcher` | `http` | `yahoo` / `equity_historical` |

Query types (`EquityHistoricalQuery`) and validation come from
`tdw-provider-fileset` so the symbol surface is consistent across providers.

## Data flow

```
transform_query (Value -> EquityHistoricalQuery)   [shared, fileset validation]
  -> extract_data (async)
       offline: synthesise one EquityHistoricalData -> JSON Bytes
       http:    GET /v8/finance/chart/{symbol}?interval&range -> Bytes
  -> transform_data (Bytes -> Vec<EquityHistoricalData>)
       offline: serde_json::from_slice (round-trips the synthetic JSON)
       http:    parse the Yahoo chart envelope, drop null bars, convert
                unix timestamps to YYYY-MM-DD via unix_to_iso_date
```

The HTTP path drops any "current"/open-session bar where Yahoo emits all-null
OHLCV fields, and dates each bar from its Unix timestamp without pulling a
date/time crate (the in-crate `unix_to_iso_date` uses Hinnant's
civil-from-days algorithm).

## Offline / cassette design

Two seams keep the network out of tests and examples:

- `YahooEquityHistoricalFetcher` is a *real* `Fetcher` that never touches the
  network — its `extract_data` builds the bytes in-process, so the full
  pipeline (including the async `extract_data`) is exercised offline. The unit
  tests and `examples/basic.rs` drive it with a no-op waker, no runtime.
- `YahooHttpEquityHistoricalFetcher::with_base_url(..)` retargets the live
  `extract_data` at a recorded-cassette HTTP server for integration tests, and
  its `transform_data` is pure over `Bytes` so the chart-envelope parser is
  tested offline. Live calls require both `http` and `TDW_YAHOO_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No captured Yahoo responses are committed; only synthetic data and fixtures
  shaped like the chart envelope appear in the crate.
- `reqwest` / `tokio` are optional, gated behind `http`; the default build is
  offline and deterministic.
- The crate talks only to Yahoo's documented public v8 chart endpoint; no
  authentication or private APIs.
