# Architecture — tdw-provider-cboe

## Module map

| Module            | Feature | Responsibility                                                                       |
| ----------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`          | always  | Constants, query structs, domain models (`CboeOptionContract`/`CboeIndexQuote`), `CboeProviderError`, request-path helpers, `stub_*`. |
| `http_fetcher.rs` | `http`  | Private `*Envelope`/`Raw*` wire structs, the two `Fetcher` impls, `From<CboeProviderError>`. |

Constants in `lib.rs`: `PROVIDER_ID = "cboe"`, `BASE_URL`. No API-key env var —
the CDN endpoints are public.

## Traits implemented

Each fetcher implements `tdw_core::Fetcher<Q, D>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impls are
hand-written.

| Fetcher                    | `Q`                | `D`                   | `PROVIDER` / `ENDPOINT`  |
| -------------------------- | ------------------ | --------------------- | ------------------------ |
| `CboeHttpOptionsFetcher`   | `CboeOptionsQuery` | `CboeOptionContract`  | `cboe` / `options`       |
| `CboeHttpIndexFetcher`     | `CboeIndexQuery`   | `CboeIndexQuote`      | `cboe` / `index_quotes`  |

Each exposes `registry_entry()` (`RegistryEntry::fetcher(PROVIDER, ENDPOINT)`) and
a `with_base_url(..)` builder for mock-server testing.

## Request → transform → data flow

1. **`transform_query(Value) -> Q`** — options reads `symbol` (alias `ticker`);
   index reads `index` (alias `symbol`). Both delegate to the query constructors:
   option symbols are upper-cased and restricted to `[A-Za-z0-9.-]` (blocks
   path/query injection); index tickers must be uppercase ASCII letters only.
2. **`extract_data(&Q, &Credentials) -> Bytes`** — builds the request path via the
   shared `options_request_path` / `index_request_path` helpers (re-validating the
   symbol), GETs `{base_url}{path}` (no credentials). Non-2xx becomes
   `Error::Provider`.
3. **`transform_data(&Q, Bytes) -> Vec<D>`** — deserialises the
   `{ "data": { ... } }` envelope into private wire structs and maps them into the
   public domain types (options yields the full contract list; index yields a
   single-element `Vec`).

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "cboe", ENDPOINT)`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio` and no `http_fetcher` unless `http` is
  enabled, so workspace builds and tests are offline.
- `stub_options_chain` / `stub_index_quote` return deterministic fixtures for
  offline use; the request-path helpers are unit-tested for injection safety.
- Under `http`, **cassette tests** feed recorded JSON byte slices into
  `transform_data` and assert on the parsed rows — exactly what
  [`examples/basic.rs`](examples/basic.rs) reproduces.
- **Live tests** are double-gated: they require `http` *and* `TDW_CBOE_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against the documented public CDN paths.
- Wire structs are private; only the mapped domain types cross the boundary.
