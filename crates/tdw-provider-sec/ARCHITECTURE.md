# tdw-provider-sec — Architecture

## Module map

| Module | Gate | Responsibility |
| ------ | ---- | -------------- |
| `lib.rs` | always | Query types (`SecHistoricalQuery`, `SecFilingsQuery`), error enum (`SecProviderError`), validation helpers, and the `BASE_URL` / `PROVIDER_ID` constants. Pure, offline, no IO. |
| `http_fetcher.rs` | `feature = "http"` | The two production `Fetcher` implementations plus their private wire-shape structs and the shared `build_client` helper. |

`lib.rs` re-exports `SecFilingsHttpFetcher` and `SecXbrlHttpFetcher` only when
`http` is enabled, so the public surface shrinks cleanly in offline builds.

## Traits implemented

Both fetchers implement `tdw_core::Fetcher<Q, D>`:

| Type | `Q` (query) | `D` (row) | `PROVIDER` / `ENDPOINT` |
| ---- | ----------- | --------- | ----------------------- |
| `SecFilingsHttpFetcher` | `SecFilingsQuery` | `SecFiling` | `sec` / `filings` |
| `SecXbrlHttpFetcher` | `SecHistoricalQuery` | `MarketDataBar` | `sec` / `xbrl_revenue` |

`Fetcher` is a three-stage pipeline; the default `fetch()` method chains them:

```
transform_query (Value -> Q)  ->  extract_data (Q -> Bytes, async IO)
                              ->  transform_data (Bytes -> Vec<D>, pure)
```

## Data flow

1. `transform_query` parses a `serde_json::Value` into a validated query.
   Symbols are upper-cased; CIKs are checked for digits and exposed via
   `padded_cik()` (zero-padded to 10, EDGAR's required path form).
2. `extract_data` builds the EDGAR URL, sends a `reqwest` GET with a
   descriptive `User-Agent`, checks status, reads the body, then sleeps
   `RATE_LIMIT_DELAY` (100 ms) to respect EDGAR's ~10 req/sec ceiling.
3. `transform_data` deserialises the JSON envelope into private wire structs
   and flattens to public rows. The XBRL fetcher keeps only `10-K` annual
   `us-gaap/Revenue` USD facts and maps each into a `MarketDataBar`
   (`source = "sec-xbrl"`, `venue = "sec"`).

## Offline / cassette design

`transform_data` is **pure** and feature-independent of the network: it
accepts `Bytes` and returns rows. Every unit/integration test exercises it
against inline JSON "cassettes" that mirror real EDGAR payloads, so the full
parsing path is covered with zero network access. `with_base_url(..)` lets a
test point `extract_data` at a local stub server. Live calls only happen when
both the `http` feature is compiled **and** `TDW_SEC_LIVE=1` is set.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendored or scraped SEC data lives in the crate; only synthetic
  fixtures shaped like the public schema appear in tests and the example.
- Network dependencies (`reqwest`, `tokio`) are optional and gated, so the
  default build is offline and deterministic.
- The provider talks only to the documented public EDGAR API; no
  authentication, cookies, or private endpoints are used.
