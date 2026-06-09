# Architecture — tdw-provider-adanos

## Module map

| Module                | Feature | Responsibility                                                                       |
| --------------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`              | always  | Provider constants, `AdanosProviderError`, query structs, domain models, `mock_fetch_*`. |
| `http_fetcher.rs`     | `http`  | Wire-format `Wire*` structs, shared HTTP helpers, the three `Fetcher` impls.         |
| `tests/http_fetcher.rs` | `http` | Cassette deserialisation tests, missing-key test, `TDW_ADANOS_LIVE`-gated live tests. |

Constants live in `lib.rs`: `PROVIDER_ID = "adanos"`, `BASE_URL`,
`API_KEY_ENV = "TDW_ADANOS_API_KEY"`.

## Traits implemented

Each fetcher implements `tdw_core::Fetcher<Q, D>` directly (this repo has no
`provider_fetcher_struct!` macro or `ProviderSpec`; fetchers are hand-written):

| Fetcher                        | `Q`                       | `D`                       | `PROVIDER` / `ENDPOINT`    |
| ------------------------------ | ------------------------- | ------------------------- | -------------------------- |
| `AdanosSentimentHttpFetcher`   | `AdanosSentimentQuery`    | `AdanosSentimentResult`   | `adanos` / `sentiment`     |
| `AdanosTrendingHttpFetcher`    | `AdanosTrendingQuery`     | `AdanosTrendingItem`      | `adanos` / `trending`      |
| `AdanosPolymarketHttpFetcher`  | `AdanosPolymarketQuery`   | `AdanosPolymarketEvent`   | `adanos` / `polymarket`    |

Each also exposes `registry_entry() -> RegistryEntry` via
`RegistryEntry::fetcher(PROVIDER, ENDPOINT)` and a `with_base_url(..)` builder for
pointing tests at a mock server.

## Request → transform → data flow

`Fetcher::fetch` (default method on the trait) chains the three steps:

1. **`transform_query(Value) -> Q`** — deserialises and re-validates the JSON
   params. Validation is centralised in the query constructors (`*::new`):
   tickers are trimmed, upper-cased, and rejected if empty or containing
   characters outside `[A-Za-z0-9._-]` (blocks path/query injection); limits are
   bounded (`trending` ≤ 100, `polymarket` ≤ 50).
2. **`extract_data(&Q, &Credentials) -> Bytes`** — reads `TDW_ADANOS_API_KEY`,
   builds a `reqwest` client with the crate `User-Agent`, sends the request with
   the `X-API-Key` header, and returns the raw response body. Non-2xx responses
   become `Error::Provider` with the status and body.
3. **`transform_data(&Q, Bytes) -> Vec<D>`** — parses the bytes into private
   `Wire*` structs (which carry the JSON `camelCase`/rename mapping), then maps
   them into the public domain types via the `map_*` helpers.

The result is wrapped in `OBBject::new(rows, PROVIDER, ENDPOINT)`.

## Offline-default + cassette design

- `default = []`: with no features the crate has no `reqwest`/`tokio` dependency
  and `http_fetcher` is not compiled, so workspace builds and tests are offline.
- The `mock_fetch_*` functions in `lib.rs` return deterministic fixtures for
  unit tests and offline development without touching `http`.
- Under `http`, the **cassette tests** never hit the network: they feed recorded
  JSON byte slices straight into `transform_data` and assert on the parsed rows.
  This is exactly what [`examples/basic.rs`](examples/basic.rs) reproduces.
- **Live tests** are double-gated: they require the `http` feature *and*
  `TDW_ADANOS_LIVE=1`, so unattended CI never makes external calls.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK is used — only `reqwest` against documented REST paths.
- The missing-key test avoids mutating process env (which would need `unsafe`)
  and instead skips when a key is already present.
- Wire structs are private; only mapped domain types cross the crate boundary.
