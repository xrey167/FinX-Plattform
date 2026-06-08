# tdw-provider-trading-economics — Architecture

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query types, data models (`TradingEconomicsCalendarEvent`, `TradingEconomicsIndicatorRow`), error enum, validation, the offline stub fetchers, and the `PROVIDER_ID` / `BASE_URL` / `API_KEY_ENV` constants. |
| `http_fetcher.rs` | `feature = "http"` | The two real `Fetcher` implementations and their HTTP plumbing. |

## Traits implemented

The real fetchers implement `tdw_core::Fetcher<Q, D>`:

| Type | `Q` | `D` | `PROVIDER` / `ENDPOINT` |
| ---- | --- | --- | ----------------------- |
| `TradingEconomicsHttpCalendarFetcher` | `TradingEconomicsCalendarQuery` | `TradingEconomicsCalendarEvent` | `trading_economics` / `calendar` |
| `TradingEconomicsHttpIndicatorFetcher` | `TradingEconomicsIndicatorQuery` | `TradingEconomicsIndicatorRow` | `trading_economics` / `indicator` |

The mock fetchers (`TradingEconomicsMockCalendarFetcher`,
`TradingEconomicsMockIndicatorFetcher`) are plain structs exposing
`fetch_stub(&Q) -> Result<Vec<D>>`; they are not `Fetcher`s and need no async
runtime.

## Data flow

```
Real path:  transform_query -> extract_data (async HTTP) -> transform_data
Stub path:  fetch_stub (return one hardcoded row)          [sync, offline]
```

1. Queries validate up front: calendar takes an `importance_min` (0..=3);
   indicator validates `country` and `indicator` slug characters.
2. The real fetchers append `TDW_TRADING_ECONOMICS_API_KEY` to the request and
   map the Trading Economics JSON arrays into the public row models.
3. The stub fetchers return one deterministic, schema-shaped row each, mirroring
   the real fetcher signature.

## Offline / mock design

`fetch_stub` is the offline seam: it produces deterministic rows (a high-
importance Non-Farm-Payrolls event; a US GDP growth indicator row) with no IO
and no feature flag, so `examples/basic.rs` and the unit tests run under the
default (offline) build. Live network access requires the `http` feature and a
real API key.

## Clean-room invariants

- `#![forbid(unsafe_code)]` via workspace lints.
- No captured Trading Economics responses are committed; only synthetic stub
  rows and fixtures shaped like the documented schema appear in the crate.
- `reqwest` / `tokio` are optional, gated behind `http`; the default build is
  offline and deterministic.
- The crate talks only to documented Trading Economics REST endpoints.
