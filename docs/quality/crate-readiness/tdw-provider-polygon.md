# tdw-provider-polygon Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-provider-polygon\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependency shape match an offline provider request contract.
- [x] Dependency direction reviewed: no local dependencies or reverse local consumers.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: request builder rejects missing API key, empty tickers, and path/query-unsafe ticker characters.
- [x] Runtime behavior reviewed: builds typed Polygon aggregate request metadata without performing live network calls or storing secrets.
- [x] Tests and coverage evidence recorded: test covers provider/path/credential metadata, normalization, missing key, empty ticker, and query injection rejection.
- [x] Docs and examples reviewed: worksheet records the provider contract; no separate README/examples required.
- [x] Surface wiring reviewed: no higher-level crate currently depends directly on this provider.
- [x] Scaffold, dead-code, and fallback signals classified: former stub signal removed; remaining match is a test-only panic helper.
- [x] Security and reliability risks reviewed: credential presence is explicit and untrusted ticker input cannot alter the request path/query.

## Findings

- Polygon provider is an offline request-contract crate, not a live data client.
- Ticker validation now rejects query/path injection characters before composing aggregate paths.
- Follow-up boundary: HTTP execution, pagination, adjusted/split policy, rate limits, and secret loading belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production Polygon transport/runtime integration.

## Production Backend Evidence (G011)

`PolygonHttpAggregatesFetcher` (gated by `--features http`) lives in
`crates/tdw-provider-polygon/src/http_fetcher.rs` and implements
`tdw_core::Fetcher` against Polygon's stock aggregates endpoint
directly via `reqwest`. No SDK required; live calls load the API key
from `POLYGON_API_KEY`.

Existing `aggregates_request` keeps the request-contract surface and
ticker validation for offline tests and downstream callers.

Public surface:
- `PolygonAggregatesQuery::new(ticker, from, to)` — validates and
  normalizes tickers plus `YYYY-MM-DD` path dates.
- `with_adjusted(adjusted)` / `with_limit(limit)` — configure
  Polygon's adjusted-price and result-limit query parameters.
- `PolygonHttpAggregatesFetcher::default()` — base URL
  `https://api.polygon.io`.
- `with_base_url(url)` — point at an alternate Polygon-compatible
  endpoint.
- `Fetcher::transform_query` accepts `{ "ticker": "MSFT", "from":
  "2024-01-02", "to": "2024-01-05" }`; `symbol` is also accepted as a
  ticker alias.
- `Fetcher::extract_data` issues `GET
  /v2/aggs/ticker/{ticker}/range/1/day/{from}/{to}` with `adjusted`,
  `sort=asc`, `limit`, and `apiKey` query parameters.
- `Fetcher::transform_data` parses Polygon's aggregates envelope into
  `tdw_domain::MarketDataBar` rows with day granularity.

Tests (`crates/tdw-provider-polygon/tests/http_fetcher.rs`,
double-gated by `--features http`):
- `cassette_replay_decodes_polygon_aggregates_into_market_bars` —
  always runs under the feature; parses a recorded Polygon response
  shape and asserts OHLCV row decoding.
- `cassette_replay_surfaces_polygon_error_envelope` — propagates
  Polygon's JSON error envelope as `Error::Provider`.
- `transform_query_normalizes_ticker_and_rejects_path_injection` —
  keeps the existing path/query-injection boundary active on the HTTP
  fetcher.
- `live_polygon_returns_recent_bars_when_env_vars_set` —
  additionally gated by `TDW_POLYGON_LIVE=1`; requires
  `POLYGON_API_KEY` and performs a real HTTP request to Polygon.

See `docs/quality/production-transport-status.md` for the broader
G011 punch list.
