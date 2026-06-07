# Architecture — tdw-provider-oecd

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | `OecdQuery`, `OecdObservation`, `OecdProviderError`, the `ProviderEndpoint` metadata type, `endpoints()`, dataset validation, the `sdmx_data_url` builder, and the `MockOecdFetcher`. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `OecdHttpDataFetcher` and the private SDMX-JSON deserialisation types (`SdmxEnvelope` and friends). |

Public constants: `BASE_URL`, `PROVIDER_ID = "oecd"`.

## Traits

`OecdHttpDataFetcher` implements `tdw_core::Fetcher<OecdQuery, OecdObservation>`:

- `const PROVIDER = "oecd"`, `const ENDPOINT = "sdmx_data"`.
- `transform_query` requires `dataset`, `filter`, `start_time`, `end_time` and
  re-validates via `OecdQuery::new` (dataset: ASCII alphanumeric/underscore,
  ≤50 chars; filter must be non-empty).
- `extract_data` builds the URL via `build_url` (honouring `with_base_url`) and
  issues a no-auth `GET`.
- `transform_data` decodes the SDMX-JSON envelope.

`registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No
`provider_fetcher_struct!` macro; the struct is hand-written for `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ OecdQuery
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (SDMX-JSON)
                                     │
                                     ▼ transform_data
                              Vec<OecdObservation>
```

`transform_data` reads the `TIME_PERIOD` dimension values from
`structure/dimensions/observation`, then maps each `dataSets[0].observations`
entry — keyed `"i:j:k:t"` — by parsing the **last** colon component as the
time index. Index 0 of each observation array is the numeric value; null values
are skipped. Rows are sorted by key for deterministic output.

## Offline / cassette design

The `sdmx_data_url` builder, dataset validation, and `MockOecdFetcher` live in
`lib.rs` and are unit-tested without the `http` feature. `transform_data` is
pure, so cassette tests feed recorded SDMX-JSON `Bytes` with no network;
`examples/basic.rs` drives `transform_query` + `transform_data` over an inline
fixture. The live test is gated by `TDW_OECD_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The SDMX-JSON serde types mirror only the public OECD wire format; no vendor
  SDK is vendored.
- Network access lives solely behind the `http` feature.
- The dataset code is re-validated even on an already-built query so a hand-made
  `OecdQuery` cannot smuggle an invalid code into the URL. Errors map into
  `tdw_core::Error::{InvalidQuery, Provider}`.
