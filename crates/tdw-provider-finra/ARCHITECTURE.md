# Architecture — tdw-provider-finra

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query models (`FinraShortInterestQuery`, `FinraOtcSummaryQuery`), record models (`FinraShortInterestRecord`, `FinraOtcSummaryRecord`), `FinraProviderError`, the pipe-delimited row/response parsers, and the `MockShortInterestFetcher` / `MockOtcSummaryFetcher` helpers. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `FinraShortInterestHttpFetcher`, `FinraOtcSummaryHttpFetcher`, and a shared `reqwest` client builder. |

Public constants: `PROVIDER_ID = "finra"`, `BASE_URL`,
`SHORT_INTEREST_MAX_LIMIT = 1000`.

## Traits

Both fetchers implement `tdw_core::Fetcher`:

| Fetcher | `Q` | `D` | `ENDPOINT` |
| ------- | --- | --- | ---------- |
| `FinraShortInterestHttpFetcher` | `FinraShortInterestQuery` | `FinraShortInterestRecord` | `short_interest` |
| `FinraOtcSummaryHttpFetcher` | `FinraOtcSummaryQuery` | `FinraOtcSummaryRecord` | `otc_summary` |

`const PROVIDER = "finra"`. `transform_query` reads `limit`/`offset` from JSON
with sensible defaults (25/0 and 10) and clamps via the typed constructor.
`registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. Structs
are hand-written (no `provider_fetcher_struct!` macro) to permit `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ Query (limit-validated)
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (pipe-delimited text)
                                     │
                                     ▼ transform_data
                              Vec<Record>
```

Unlike the JSON providers, FINRA returns pipe-delimited rows. `transform_data`
validates UTF-8, then delegates to the shared `parse_*_response` functions,
which split on `|`, enforce the exact field count, parse numeric columns, and
skip blank lines. An empty body decodes to an empty `Vec` (not an error).

## Offline / cassette design

The parsers live in `lib.rs` and are the heart of the provider, so they are
unit-tested directly without the `http` feature. The `http_fetcher.rs` tests
feed recorded pipe-delimited `Bytes` through `transform_data` (cassette replay)
with no network. `examples/basic.rs` exercises the same path. The live test is
gated by `TDW_FINRA_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The wire-format parsing is derived only from the documented FINRA field
  order; no vendor SDK is vendored.
- Network access lives solely behind the `http` feature.
- Parse failures map into `tdw_core::Error::{InvalidQuery, Provider}` with the
  offending field/expected-count carried for diagnostics.
