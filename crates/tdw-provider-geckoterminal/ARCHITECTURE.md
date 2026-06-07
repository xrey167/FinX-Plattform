# Architecture — tdw-provider-geckoterminal

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query models (`GeckoTerminalPoolQuery`, `GeckoTerminalTrendingQuery`, `GeckoTerminalTokenPoolsQuery`), the `DexPool` data model, `GeckoTerminalError`, network/address validation, and the `mock_pool` stub. |
| `http_fetcher.rs` | `#[cfg(feature = "http")]` | `GeckoTerminalHttpFetcher`, private serde envelope shapes, the `gecko_data_to_pool` converter, and the standalone `fetch_trending_raw` / `fetch_token_pools_raw` / `parse_pool_list` helpers. |

Public constants: `PROVIDER_ID = "geckoterminal"`, `BASE_URL`, `ACCEPT_HEADER`.

## Traits

`GeckoTerminalHttpFetcher` implements
`tdw_core::Fetcher<GeckoTerminalPoolQuery, DexPool>`:

- `const PROVIDER = "geckoterminal"`, `const ENDPOINT = "pool"`.
- `transform_query` reads `network` + `pool_address` and validates them
  (network: lowercase alphanumeric/hyphen; address: EVM `0x…` ≥42 chars or
  Solana 32–44 chars).
- `extract_data` issues `GET /networks/{network}/pools/{pool_address}` with the
  version-pinning `Accept` header.
- `transform_data` decodes the single-pool envelope into one `DexPool`.

The trending and token-pool surfaces are exposed as **standalone async
functions** plus the shared `parse_pool_list` decoder rather than additional
`Fetcher` impls. `registry_entry()` returns
`RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No `provider_fetcher_struct!`
macro; the struct is hand-written for `with_base_url`.

## Request → transform → data flow

```
JSON params ──transform_query──▶ GeckoTerminalPoolQuery
                                     │
                                     ▼ extract_data  (HTTP, feature = "http")
                              raw Bytes (JSON:API envelope)
                                     │
                                     ▼ transform_data / gecko_data_to_pool
                              Vec<DexPool>
```

GeckoTerminal returns prices/volumes as JSON **strings** nested under
`data.attributes`; `gecko_data_to_pool` parses each into `Option<f64>`, leaving
absent fields as `None` rather than failing.

## Offline / cassette design

`transform_data` and `parse_pool_list` are pure decoders, so cassette tests feed
recorded JSON `Bytes` with no network. The `mock_pool` stub backs the offline
path (used when `http` is disabled); `examples/basic.rs` drives `transform_query`
+ `transform_data` over an inline single-pool fixture. The live test is gated by
`TDW_GECKOTERMINAL_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- The serde shapes mirror only the public GeckoTerminal JSON:API wire format; no
  vendor SDK is vendored.
- Network access lives solely behind the `http` feature.
- Address/network validation rejects path-traversal-style input before it
  reaches a URL; errors map into `tdw_core::Error::{InvalidQuery, Provider}`.
