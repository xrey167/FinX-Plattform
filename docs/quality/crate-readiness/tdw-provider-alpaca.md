# tdw-provider-alpaca Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-provider-alpaca\Cargo.toml
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
- [x] Public API and error contracts reviewed: request builder rejects missing API key, empty symbols, and path-unsafe symbol characters.
- [x] Runtime behavior reviewed: builds typed Alpaca stock-bars request metadata without performing live network calls or storing secrets.
- [x] Tests and coverage evidence recorded: test covers endpoint metadata, credential header, normalization, missing key, empty symbol, and traversal-like input rejection.
- [x] Docs and examples reviewed: worksheet records the provider contract; no separate README/examples required.
- [x] Surface wiring reviewed: no higher-level crate currently depends directly on this provider.
- [x] Scaffold, dead-code, and fallback signals classified: former stub signal removed; remaining match is a test-only panic helper.
- [x] Security and reliability risks reviewed: credential presence is explicit and untrusted symbol input cannot alter the request path.

## Findings

- Alpaca provider is an offline request-contract crate, not a live data client.
- Symbol validation now rejects path/query injection characters before composing `/v2/stocks/{symbol}/bars`.
- Follow-up boundary: HTTP execution, pagination, rate limits, and secret loading belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production Alpaca transport/runtime integration.

## Production Backend Evidence (G011)

`AlpacaHttpStockBarsFetcher` (gated by `--features http`) lives in
`crates/tdw-provider-alpaca/src/http_fetcher.rs` and implements
`tdw_core::Fetcher` against Alpaca's historical stock bars endpoint
directly via `reqwest`. No SDK required; live calls load credentials
from `APCA_API_KEY_ID` and `APCA_API_SECRET_KEY`.

Existing `stock_bars_request` keeps the request-contract surface and
symbol validation for offline tests and downstream callers.

Public surface:
- `AlpacaStockBarsQuery::new(symbol, start, end)` — validates and
  normalizes symbols plus `YYYY-MM-DD` query dates.
- `with_timeframe(timeframe)` / `with_limit(limit)` /
  `with_feed(feed)` — configure Alpaca query parameters without
  allowing query injection.
- `AlpacaHttpStockBarsFetcher::default()` — base URL
  `https://data.alpaca.markets`.
- `with_base_url(url)` — point at an alternate Alpaca-compatible
  endpoint.
- `Fetcher::transform_query` accepts `{ "symbol": "AAPL", "start":
  "2024-01-02", "end": "2024-01-05" }`.
- `Fetcher::extract_data` issues `GET /v2/stocks/bars` with `symbols`,
  `timeframe`, `start`, `end`, `limit`, optional `feed`, and Alpaca key
  / secret headers.
- `Fetcher::transform_data` parses Alpaca's per-symbol bars envelope
  into `tdw_domain::MarketDataBar` rows with day granularity.

Tests (`crates/tdw-provider-alpaca/tests/http_fetcher.rs`,
double-gated by `--features http`):
- `cassette_replay_decodes_alpaca_bars_into_market_bars` — always
  runs under the feature; parses a recorded Alpaca response shape and
  asserts OHLCV row decoding.
- `cassette_replay_surfaces_alpaca_error_envelope` — propagates
  Alpaca's JSON error envelope as `Error::Provider`.
- `transform_query_normalizes_symbol_and_rejects_path_injection` —
  keeps the existing path/query-injection boundary active on the HTTP
  fetcher.
- `live_alpaca_returns_recent_bars_when_env_vars_set` — additionally
  gated by `TDW_ALPACA_LIVE=1`; requires `APCA_API_KEY_ID` and
  `APCA_API_SECRET_KEY` and performs a real HTTP request to Alpaca.

See `docs/quality/production-transport-status.md` for the broader
G011 punch list.
