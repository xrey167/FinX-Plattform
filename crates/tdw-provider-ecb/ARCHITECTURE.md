# Architecture — tdw-provider-ecb

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Typed models (`EcbDataQuery`, `EcbObservation`), `EcbProviderError`, validation, `data_request_path`, the envelope parser `parse_ecb_json` / `parse_ecb_value`, and the offline `EcbMockFetcher`. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `EcbHttpDataFetcher`, the real `reqwest`-based `tdw_core::Fetcher` implementation. |

Public constants: `PROVIDER_ID = "ecb"`, `BASE_URL`.

## Traits

The HTTP fetcher implements [`tdw_core::Fetcher<EcbDataQuery, EcbObservation>`]:

- `const PROVIDER = "ecb"`, `const ENDPOINT = "data"`.
- `transform_query(Value) -> Result<EcbDataQuery>` — pulls `flow`, `key`,
  `start_period`, `end_period` from JSON and re-validates via
  `EcbDataQuery::new`.
- `extract_data(&self, &query, &Credentials) -> Result<Bytes>` — the only
  network step; issues `GET {base}/data/{flow}/{key}` with
  `format=jsondata`.
- `transform_data(&self, &query, Bytes) -> Result<Vec<EcbObservation>>` —
  decodes the SDW JSON envelope.

`registry_entry()` returns a `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`
for registry wiring. No `provider_fetcher_struct!` macro is used; the struct
is hand-written so the base URL can be overridden via `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ EcbDataQuery
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (jsondata)
                                     │
                                     ▼ transform_data / parse_ecb_value
                              Vec<EcbObservation>
```

`parse_ecb_value` reads the `structure/dimensions/observation` block to map
the `TIME_PERIOD` index to dates, then walks `dataSets[].series[].observations`,
emitting one `EcbObservation` per non-null value. Rows are sorted by date for
deterministic output.

## Offline / cassette design

`transform_data` is pure and side-effect free, so it is exercised directly in
the cassette tests (`tests/http_fetcher.rs`) by feeding recorded `jsondata`
bytes — no network. The `EcbMockFetcher` in `lib.rs` reuses the same
`parse_ecb_json` path so offline callers (and the `examples/basic.rs` example)
get byte-identical decoding to the live fetcher. The live test is guarded by
`TDW_ECB_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- All vendor-specific code is derived only from the public SDW wire format; no
  third-party SDK is vendored.
- Network access exists solely behind the `http` feature; default builds cannot
  reach the network.
- Errors are mapped into `tdw_core::Error::{InvalidQuery, Provider}` at the
  trait boundary, keeping provider error detail out of the core contract.
