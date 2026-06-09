# Architecture — tdw-provider-benzinga

## Module map

| Module                | Feature | Responsibility                                                                       |
| --------------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`              | always  | Constants, `BenzingaProviderError`, query structs, domain models (`BenzingaNewsItem`/`BenzingaEarningsItem`), `stub_fetch_*`. |
| `http_fetcher.rs`     | `http`  | `Wire*` structs, shared HTTP helpers, the two `Fetcher` impls, offline `transform_data` unit tests. |

Constants in `lib.rs`: `PROVIDER_ID = "benzinga"`, `BASE_URL`,
`API_KEY_ENV = "TDW_BENZINGA_API_KEY"`.

## Traits implemented

Each fetcher implements `tdw_core::Fetcher<Q, D>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impls are
hand-written.

| Fetcher                        | `Q`                      | `D`                      | `PROVIDER` / `ENDPOINT`  |
| ------------------------------ | ------------------------ | ------------------------ | ------------------------ |
| `BenzingaNewsHttpFetcher`      | `BenzingaNewsQuery`      | `BenzingaNewsItem`       | `benzinga` / `news`      |
| `BenzingaEarningsHttpFetcher`  | `BenzingaEarningsQuery`  | `BenzingaEarningsItem`   | `benzinga` / `earnings`  |

Each exposes `registry_entry()` (`RegistryEntry::fetcher(PROVIDER, ENDPOINT)`) and
a `with_base_url(..)` builder for mock-server testing.

## Request → transform → data flow

1. **`transform_query(Value) -> Q`** — extracts and re-validates params via the
   query constructors (`*::new`): symbols are trimmed, upper-cased, and rejected
   if empty or containing characters outside `[A-Za-z0-9._-]`; news `page_size`
   must be `1..=100`; earnings dates must be `YYYY-MM-DD`.
2. **`extract_data(&Q, &Credentials) -> Bytes`** — reads `TDW_BENZINGA_API_KEY`,
   builds a `reqwest` client, and GETs the endpoint with the
   `Authorization: Token <key>` header and the appropriate query params
   (`tickers`/`pageSize`/`displayOutput` for news; `tickers`/`dateFrom`/`dateTo`
   for earnings). Non-2xx becomes `Error::Provider`.
3. **`transform_data(&Q, Bytes) -> Vec<D>`** — parses the bytes into private
   `Wire*` structs (news is a top-level JSON array; earnings is an
   `{ "earnings": [...] }` envelope), then maps them into the public domain types
   via `map_news_item` / `map_earnings_item` (the news `stocks` array of
   `{name}` objects is flattened into a `Vec<String>`).

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "benzinga", ENDPOINT)`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio` and no `http_fetcher` unless `http` is
  enabled, so workspace builds and tests are offline.
- `stub_fetch_news` / `stub_fetch_earnings` return deterministic fixtures for
  offline use without `http`.
- Under `http`, the **inline `transform_data` tests** feed recorded JSON byte
  slices straight into `transform_data` and assert on the parsed rows — exactly
  what [`examples/basic.rs`](examples/basic.rs) reproduces.
- **Live tests** are double-gated: they require `http` *and* `TDW_BENZINGA_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against documented REST paths.
- Wire structs are private; only mapped domain types cross the boundary.
