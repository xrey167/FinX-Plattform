# Architecture — tdw-provider-deribit

## Module map

| Module            | Feature | Responsibility                                                                       |
| ----------------- | ------- | ----------------------------------------------------------------------------------- |
| `lib.rs`          | always  | Constants, `DeribitKind`, three query structs + validation, domain models, `DeribitProviderError`, request-path helpers, `stub_*`. |
| `http_fetcher.rs` | `http`  | Private `Deribit Envelope<T>` + `Raw*` wire structs, the three `Fetcher` impls, `parse_kind`, `From<DeribitProviderError>`. |

Constants in `lib.rs`: `PROVIDER_ID = "deribit"`, `BASE_URL`. No API-key env var —
the endpoints are public.

## Traits implemented

Each fetcher implements `tdw_core::Fetcher<Q, D>` directly — there is no
`provider_fetcher_struct!` macro or `ProviderSpec` in this repo; the impls are
hand-written.

| Fetcher                          | `Q`                        | `D`                    | `PROVIDER` / `ENDPOINT`    |
| -------------------------------- | -------------------------- | ---------------------- | -------------------------- |
| `DeribitHttpInstrumentsFetcher`  | `DeribitInstrumentsQuery`  | `DeribitInstrument`    | `deribit` / `instruments`  |
| `DeribitHttpOrderBookFetcher`    | `DeribitOrderBookQuery`    | `DeribitOrderBook`     | `deribit` / `order_book`   |
| `DeribitHttpFundingFetcher`      | `DeribitFundingQuery`      | `DeribitFundingRecord` | `deribit` / `funding_rate` |

Each exposes `registry_entry()` and a `with_base_url(..)` builder for
mock-server testing.

## Request → transform → data flow

1. **`transform_query(Value) -> Q`** — reads each fetcher's params and delegates
   to the query constructors: currencies are uppercase ASCII letters only;
   instrument names are restricted to `[A-Za-z0-9._-]` (blocks `&depth=`
   injection); order-book `depth` ∈ {1,5,10,20} (default 5); funding `count`
   ∈ 1..=1000 (default 100) with `end_ms ≥ start_ms`. Unknown `kind` strings are
   rejected by `parse_kind`.
2. **`extract_data(&Q, &Credentials) -> Bytes`** — builds the request path via the
   shared `*_request_path` helpers (re-validating), GETs `{base_url}{path}` (no
   credentials). Non-2xx becomes `Error::Provider`.
3. **`transform_data(&Q, Bytes) -> Vec<D>`** — parses the JSON-RPC-style
   `{ "result": ... }` envelope (`DeribitEnvelope<T>`) into private `Raw*` structs
   and maps them into the public domain types (instruments and funding yield
   lists; order book yields a single-element `Vec` with nested `DeribitGreeks`).

`Fetcher::fetch` wraps the rows in `OBBject::new(rows, "deribit", ENDPOINT)`.

## Offline-default + cassette design

- `default = []`: no `reqwest`/`tokio`/`tdw-core`/`tdw-domain` and no
  `http_fetcher` unless `http` is enabled, so workspace builds and tests are
  offline. (`serde_json` is non-optional.)
- `stub_instruments` / `stub_order_book` / `stub_funding_records` return
  deterministic fixtures, and the request-path helpers are unit-tested for
  injection safety.
- Under `http`, **cassette tests** feed recorded `{ "result": ... }` byte slices
  into `transform_data` and assert on the parsed rows — exactly what
  [`examples/basic.rs`](examples/basic.rs) reproduces.
- **Live tests** are double-gated: they require `http` *and* `TDW_DERIBIT_LIVE=1`.

## Clean-room invariants

- `#![forbid(unsafe_code)]` at the crate root.
- No vendor SDK — plain `reqwest` against the documented public v2 paths.
- Wire structs (`Deribit Envelope`, `Raw*`) are private; only the mapped domain
  types cross the boundary.
