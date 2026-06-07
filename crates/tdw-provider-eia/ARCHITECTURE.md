# Architecture — tdw-provider-eia

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | `EiaCommodity`, query models (`EiaSpotPriceQuery`, `EiaNaturalGasQuery`), record models (`EiaSpotPriceRecord`, `EiaNaturalGasRecord`), `EiaProviderError`, length validation, and the `mock_spot_price` / `mock_natural_gas` stubs. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `EiaHttpSpotPriceFetcher`, `EiaHttpNaturalGasFetcher`, shared `reqwest` client/api-key helpers, and private serde envelope shapes. |

Public constants: `PROVIDER_ID = "eia"`, `BASE_URL`, `API_KEY_PARAM`,
`API_KEY_ENV = "TDW_EIA_API_KEY"`.

## Traits

Both fetchers implement `tdw_core::Fetcher`:

| Fetcher | `Q` | `D` | `ENDPOINT` |
| ------- | --- | --- | ---------- |
| `EiaHttpSpotPriceFetcher` | `EiaSpotPriceQuery` | `EiaSpotPriceRecord` | `spot_price` |
| `EiaHttpNaturalGasFetcher` | `EiaNaturalGasQuery` | `EiaNaturalGasRecord` | `natural_gas` |

`const PROVIDER = "eia"` for both. `transform_query` deserialises the JSON via
serde and re-validates `length` through the typed constructor. `registry_entry()`
returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. The structs are
hand-written (no `provider_fetcher_struct!` macro) to allow `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ Query (length-validated)
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (JSON envelope)
                                     │
                                     ▼ transform_data
                              Vec<Record>
```

The EIA API returns numeric values as JSON **strings**; `transform_data`
deserialises the `response.data[]` array, skips empty / `"."` placeholders, and
parses the remainder into `f64`. A missing `period` is a hard error.

## Offline / cassette design

`transform_data` is pure, so cassette tests (`tests/http_fetcher.rs`) feed
recorded envelope bytes and assert decoded rows without any network. The
`mock_spot_price` / `mock_natural_gas` stubs provide the offline path used by
`examples/basic.rs` and by builds with `http` disabled. The live tests are
gated by `TDW_EIA_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The serde envelope shapes mirror only the public EIA v2 wire format; no vendor
  SDK is vendored.
- Network access lives solely behind the `http` feature.
- The API key is read from `TDW_EIA_API_KEY` at request time and never logged;
  errors are mapped into `tdw_core::Error::{InvalidQuery, Provider}`.
