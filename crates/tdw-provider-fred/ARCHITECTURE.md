# Architecture — tdw-provider-fred

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | `FredSeriesObservationsQuery`, `FredObservation`, `FredProviderError`, the `ProviderEndpoint` / `ProviderRequest` contract types, `endpoints()`, the `series_observations_request` URL/contract builder, and series-id validation. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `FredHttpSeriesObservationsFetcher` (with bounded-window builders) and the private serde envelope. |

Public constants: `PROVIDER_ID = "fred"`, `BASE_URL`, `API_KEY_PARAM`.
The HTTP module reads the API key from `FRED_API_KEY` (vendor-conventional
name, declared privately in `http_fetcher.rs`).

## Traits

`FredHttpSeriesObservationsFetcher` implements
`tdw_core::Fetcher<FredSeriesObservationsQuery, FredObservation>`:

- `const PROVIDER = "fred"`, `const ENDPOINT = "series_observations"`.
- `transform_query` reads `series_id` and validates it (uppercased; ASCII
  alphanumeric plus `_`, `-`, `.`).
- `extract_data` re-checks the request contract via `series_observations_request`,
  reads `FRED_API_KEY`, and issues `GET /series/observations` with optional
  `limit` / `observation_start` / `observation_end`.
- `transform_data` decodes the JSON envelope.

`registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No
`provider_fetcher_struct!` macro; the struct carries base-URL and window state.

## Request → transform → data flow

```
JSON { "series_id": ... } ──transform_query──▶ FredSeriesObservationsQuery
                                                    │
                                                    ▼ extract_data  (HTTP, feature = "http")
                                             raw Bytes (JSON envelope)
                                                    │
                                                    ▼ transform_data
                                             Vec<FredObservation>
```

FRED returns values as JSON **strings** and uses `"."` for missing data.
`transform_data` first surfaces any `error_code`/`error_message` from the
envelope as a `Provider` error, then parses each observation, skipping `"."` /
empty values and erroring on a missing date.

## Offline / cassette design

The request-contract builder and validation live in `lib.rs` and are unit-tested
without the `http` feature. `transform_data` is pure, so cassette-style tests can
feed recorded `Bytes`; `examples/basic.rs` drives `transform_query` +
`transform_data` over an inline fixture. The live test is gated by
`TDW_FRED_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The serde envelope mirrors only the public FRED wire format; no vendor SDK is
  vendored.
- Network access lives solely behind the `http` feature.
- The API key is read from `FRED_API_KEY` at request time; errors map into
  `tdw_core::Error::{InvalidQuery, Provider}`.
