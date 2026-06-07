# Architecture — tdw-provider-fileset

## Module map

This crate is a single module: `lib.rs`. There is no `http_fetcher.rs` and no
feature gating, because nothing here touches the network.

| Item | Responsibility |
| ---- | -------------- |
| `EquityHistoricalQuery` | Typed query (`symbol`). |
| `FilesetEquityHistoricalFetcher` | Zero-sized `tdw_core::Fetcher` implementation. |
| `fixture_rows(symbol)` | Returns the canned `Vec<EquityHistoricalData>`. |
| `normalize_symbol` (private) | Trims, uppercases, and validates the symbol. |

The output model is `tdw_domain::EquityHistoricalData` (shared workspace type),
not a crate-local struct.

## Traits

`FilesetEquityHistoricalFetcher` implements
`tdw_core::Fetcher<EquityHistoricalQuery, EquityHistoricalData>`:

- `const PROVIDER = "fileset"`, `const ENDPOINT = "equity_historical"`.
- `transform_query` reads `symbol` from JSON and normalises it.
- `extract_data` builds the fixture rows, serialises them to JSON `Bytes`, and
  returns them — this is where a network provider would do its HTTP call, but
  here it is purely in-memory.
- `transform_data` deserialises those `Bytes` back into
  `Vec<EquityHistoricalData>`.

`registry_entry()` is a `const fn` returning
`RegistryEntry::fetcher(PROVIDER, ENDPOINT)`. No `provider_fetcher_struct!`
macro is used.

## Request → transform → data flow

```
JSON { "symbol": ... } ──transform_query──▶ EquityHistoricalQuery
                                                 │
                                                 ▼ extract_data (serialise fixture)
                                          raw Bytes (JSON array)
                                                 │
                                                 ▼ transform_data (deserialise)
                                          Vec<EquityHistoricalData>
```

Routing the fixture through `extract_data`/`transform_data` (instead of
returning rows directly) keeps the data path byte-identical to the real
providers, so the same serialisation edges are exercised in tests.

## Offline / cassette design

The whole crate is the "cassette": `extract_data` is the recorded response.
This makes `fileset` the canonical offline provider for integration and
registry tests that need a working `Fetcher` without any vendor dependency.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor data, no vendor SDK, no network code — there is nothing to leak.
- Symbol validation rejects unsupported characters before they reach any path,
  matching the hardening applied to the network providers.
