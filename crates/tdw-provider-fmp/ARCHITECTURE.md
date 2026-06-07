# Architecture — tdw-provider-fmp

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query models (`FmpHistoricalQuery`, `FmpFundamentalsQuery`), the `FmpStatement` enum, record models (`FmpOhlcvRow`, `FmpIncomeRow`), `FmpError`, symbol/limit validation, and the `FmpMockHistoricalFetcher` / `FmpMockIncomeFetcher` stubs. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `FmpHttpHistoricalFetcher`, `FmpHttpIncomeFetcher`, private serde envelope shapes, and shared client/api-key helpers. |

Public constants: `PROVIDER_ID = "fmp"`, `BASE_URL`,
`API_KEY_ENV = "TDW_FMP_API_KEY"`.

## Traits

Both fetchers implement `tdw_core::Fetcher`:

| Fetcher | `Q` | `D` | `ENDPOINT` |
| ------- | --- | --- | ---------- |
| `FmpHttpHistoricalFetcher` | `FmpHistoricalQuery` | `tdw_domain::MarketDataBar` | `equity_historical` |
| `FmpHttpIncomeFetcher` | `FmpFundamentalsQuery` | `FmpIncomeRow` | `income_statement` |

`const PROVIDER = "fmp"`. `transform_query` accepts `symbol` or `ticker`,
defaults `statement` to `income` and `limit` to 5, and re-validates through the
typed constructors. The historical fetcher emits the shared `MarketDataBar`
domain type (with `venue`/`source` = `"fmp"`, daily granularity); the income
fetcher emits the crate-local `FmpIncomeRow`. `registry_entry()` returns
`RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No `provider_fetcher_struct!`
macro; structs are hand-written for `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ Query (symbol/limit-validated)
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (JSON)
                                     │
                                     ▼ transform_data
                              Vec<MarketDataBar> | Vec<FmpIncomeRow>
```

The historical endpoint wraps bars in `{ "symbol", "historical": [...] }`; the
income endpoint returns a bare array. `transform_data` falls back to the queried
symbol when the response omits it.

## Offline / cassette design

`transform_data` is pure, so cassette tests feed recorded JSON `Bytes` and
assert decoded rows with no network. The `FmpMock*Fetcher` stubs back the
offline path used when `http` is disabled; `examples/basic.rs` drives the income
fetcher's `transform_query` + `transform_data` against an inline fixture. The
live test is gated by `TDW_FMP_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The serde envelope shapes mirror only the public FMP wire format; no vendor
  SDK is vendored.
- Network access lives solely behind the `http` feature.
- The API key is read from `TDW_FMP_API_KEY` at request time; errors map into
  `tdw_core::Error::{InvalidQuery, Provider}`.
