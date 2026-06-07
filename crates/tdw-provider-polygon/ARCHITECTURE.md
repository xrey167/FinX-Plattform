# Architecture — tdw-provider-polygon

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | `PolygonAggregatesQuery` (with `with_adjusted` / `with_limit`), `PolygonProviderError`, the `ProviderRequest` contract type, ticker/date validation, and the `aggregates_request` builder. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `PolygonHttpAggregatesFetcher`, private serde envelope shapes, and the `unix_millis_to_iso_timestamp` helper (no chrono dependency). |

Public constants: `PROVIDER_ID = "polygon"`, `BASE_URL`,
`API_KEY_PARAM = "apiKey"`. The API-key env name (`POLYGON_API_KEY`) is
declared privately in `http_fetcher.rs`.

## Traits

`PolygonHttpAggregatesFetcher` implements
`tdw_core::Fetcher<PolygonAggregatesQuery, tdw_domain::MarketDataBar>`:

- `const PROVIDER = "polygon"`, `const ENDPOINT = "aggregates"`.
- `transform_query` accepts `ticker`/`symbol`, `from`, `to`, optional
  `adjusted` (default true) and `limit` (default 5000), re-validating through the
  typed constructor and builder methods.
- `extract_data` re-checks the request contract, reads `POLYGON_API_KEY`, and
  issues the dated aggregates `GET` with `adjusted` / `sort` / `limit` / `apiKey`.
- `transform_data` surfaces a `status == "ERROR"` envelope as a `Provider` error,
  then maps each `results[]` aggregate into a `MarketDataBar`.

`registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No
`provider_fetcher_struct!` macro; the struct is hand-written for `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ PolygonAggregatesQuery
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (JSON)
                                     │
                                     ▼ transform_data
                              Vec<MarketDataBar>
```

Polygon timestamps are Unix milliseconds; `unix_millis_to_iso_timestamp`
converts them to an ISO-8601 string with a self-contained civil-date algorithm
(no extra date crate), keeping the dependency surface minimal.

## Offline / cassette design

The request-contract builder and ticker/date validation live in `lib.rs` and are
unit-tested without the `http` feature. `transform_data` and the timestamp helper
are pure, so cassette tests feed recorded `Bytes`; `examples/basic.rs` drives
`transform_query` + `transform_data` over an inline aggregates fixture. The live
test is gated by `TDW_POLYGON_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The serde envelope mirrors only the public Polygon wire format; no vendor SDK
  is vendored, and the timestamp conversion avoids third-party date code.
- Network access lives solely behind the `http` feature.
- Ticker validation rejects query-injection-style input before it reaches a URL;
  the API key is read from `POLYGON_API_KEY` at request time. Errors map into
  `tdw_core::Error::{InvalidQuery, Provider}`.
