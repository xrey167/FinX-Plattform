# Architecture — tdw-provider-glassnode

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | The `GlassnodeMetric` enum (with `api_path`), `GlassnodeMetricQuery`, `GlassnodeDataPoint`, `GlassnodeProviderError`, asset/interval validation, and the `mock_fetch` stub. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `GlassnodeHttpFetcher` and the private `RawPoint` (`{t, v}`) serde shape. |

Public constants: `PROVIDER_ID = "glassnode"`, `BASE_URL`, `API_KEY_PARAM`,
`API_KEY_ENV = "TDW_GLASSNODE_API_KEY"`.

## Traits

`GlassnodeHttpFetcher` implements
`tdw_core::Fetcher<GlassnodeMetricQuery, GlassnodeDataPoint>`:

- `const PROVIDER = "glassnode"`, `const ENDPOINT = "metric"`.
- `transform_query` reads `asset`, `interval`, and a `metric` (deserialised from
  the `GlassnodeMetric` snake_case tag) and re-validates via the constructor.
- `extract_data` reads `TDW_GLASSNODE_API_KEY`, builds the URL from
  `metric.api_path()`, and sends `a` / `i` / `api_key` query params.
- `transform_data` decodes the `[{ "t", "v" }]` array, attaching the queried
  asset/metric to each point.

`registry_entry()` returns `RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No
`provider_fetcher_struct!` macro; the struct is hand-written for `with_base_url`
(and has a `new()` convenience constructor).

## Request → transform → data flow

```
JSON params ──transform_query──▶ GlassnodeMetricQuery
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (JSON array)
                                     │
                                     ▼ transform_data
                              Vec<GlassnodeDataPoint>
```

The metric is constrained to the `GlassnodeMetric` enum, so only the three
vetted endpoints can be reached; the API path is never built from raw caller
input.

## Offline / cassette design

`transform_data` is pure, so cassette tests feed recorded `[{t,v}]` `Bytes` with
no network. The `mock_fetch` stub backs the offline path (used when `http` is
disabled); `examples/basic.rs` drives `transform_query` + `transform_data` over
an inline fixture. The live test is gated by `TDW_GLASSNODE_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The `RawPoint` serde shape mirrors only the public Glassnode wire format; no
  vendor SDK is vendored.
- Network access lives solely behind the `http` feature.
- Only enum-bounded metric paths are reachable; the API key is read from
  `TDW_GLASSNODE_API_KEY` at request time. Errors map into
  `tdw_core::Error::{InvalidQuery, Provider}`.
