# Architecture — tdw-provider-nasdaq

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | `NasdaqDatasetQuery` (with optional date bounds), `NasdaqDataRow`, `NasdaqProviderError`, the `ProviderRequest` contract type, identifier/date validation, the `dataset_request` builder, and the `NasdaqStubFetcher`. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `NasdaqHttpDatasetFetcher` and private serde envelope shapes (`dataset_data` / `quandl_error`). |

Public constants: `PROVIDER_ID = "nasdaq"`, `BASE_URL`, `API_KEY_PARAM`.
The API-key env name (`TDW_NASDAQ_API_KEY`) is declared privately in
`http_fetcher.rs`.

## Traits

`NasdaqHttpDatasetFetcher` implements
`tdw_core::Fetcher<NasdaqDatasetQuery, NasdaqDataRow>`:

- `const PROVIDER = "nasdaq"`, `const ENDPOINT = "datasets"`.
- `transform_query` reads `database` + `dataset` and optional
  `start_date`/`end_date`, validating identifiers (uppercased ASCII
  alphanumeric/underscore, ≤50 chars) and dates (`YYYY-MM-DD`).
- `extract_data` re-checks the request contract, reads `TDW_NASDAQ_API_KEY`, and
  issues `GET /datasets/{database}/{dataset}/data` with optional date params.
- `transform_data` surfaces a `quandl_error` envelope as a `Provider` error, then
  fans `dataset_data.data[][]` rows out into `NasdaqDataRow`s (carrying
  `column_names`).

`registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No
`provider_fetcher_struct!` macro; the struct is hand-written for `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ NasdaqDatasetQuery
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (JSON envelope)
                                     │
                                     ▼ transform_data
                              Vec<NasdaqDataRow>
```

Each `NasdaqDataRow` keeps the raw heterogeneous `values` array
(`Vec<serde_json::Value>`) alongside the `column_names`, so consumers can map
columns without the provider imposing a fixed schema.

## Offline / cassette design

The request-contract builder and validation live in `lib.rs` and are unit-tested
without the `http` feature. `transform_data` is pure, so cassette tests feed
recorded `Bytes`; the `NasdaqStubFetcher` backs the offline path (used when
`http` is disabled). `examples/basic.rs` drives `transform_query` +
`transform_data` over an inline fixture. The live test is gated by
`TDW_NASDAQ_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The serde envelope mirrors only the public NASDAQ Data Link wire format; no
  vendor SDK is vendored.
- Network access lives solely behind the `http` feature.
- Identifier validation rejects path-traversal / query-injection input before it
  reaches a URL; the API key is read from `TDW_NASDAQ_API_KEY` at request time.
  Errors map into `tdw_core::Error::{InvalidQuery, Provider}`.
