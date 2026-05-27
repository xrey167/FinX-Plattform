# tdw-provider-binance Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-provider-binance\Cargo.toml
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
- [x] Public API and error contracts reviewed: request builder rejects empty symbols and query-unsafe symbol characters.
- [x] Runtime behavior reviewed: builds public Binance ticker-price request metadata without credentials or live network calls.
- [x] Tests and coverage evidence recorded: test covers endpoint metadata, normalization, public credential mode, empty symbol, and query injection rejection.
- [x] Docs and examples reviewed: worksheet records the provider contract; no separate README/examples required.
- [x] Surface wiring reviewed: no higher-level crate currently depends directly on this provider.
- [x] Scaffold, dead-code, and fallback signals classified: former stub signal removed; remaining match is a test-only panic helper.
- [x] Security and reliability risks reviewed: ticker query parameter is constrained to ASCII alphanumeric symbols.

## Findings

- Binance provider is an offline public-market request contract with no secret requirement.
- Symbol validation rejects query delimiter injection before composing `/api/v3/ticker/price`.
- Follow-up boundary: HTTP execution, exchange error decoding, retries, and rate limits belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production Binance transport/runtime integration.

## Production Backend Evidence (G011)

`BinanceHttpTickerPriceFetcher` (gated by `--features http`) lives in
`crates/tdw-provider-binance/src/http_fetcher.rs` and implements
`tdw_core::Fetcher` against Binance's public `/api/v3/ticker/price`
endpoint directly via `reqwest`. No SDK or credentials required for
this public market-data endpoint.

Existing `ticker_price_request` keeps the request-contract surface and
symbol validation for offline tests and downstream callers.

Public surface:
- `BinanceTickerPriceQuery::new(symbol)` — validates and normalizes
  Binance symbols.
- `BinanceTickerPrice` — decoded row shape with `symbol` and numeric
  `price`.
- `BinanceHttpTickerPriceFetcher::default()` — base URL
  `https://api.binance.com`.
- `with_base_url(url)` — point at an alternate Binance-compatible
  endpoint.
- `Fetcher::transform_query` accepts `{ "symbol": "BTCUSDT" }`.
- `Fetcher::extract_data` issues `GET /api/v3/ticker/price` with the
  normalized `symbol` query parameter.
- `Fetcher::transform_data` parses Binance's string price into `f64`
  and propagates Binance `code` / `msg` error envelopes as
  `Error::Provider`.

Tests (`crates/tdw-provider-binance/tests/http_fetcher.rs`,
double-gated by `--features http`):
- `cassette_replay_decodes_binance_ticker_price` — always runs under
  the feature; parses a recorded Binance response shape and asserts
  price decoding.
- `cassette_replay_surfaces_binance_error_envelope` — propagates
  Binance's JSON error envelope as `Error::Provider`.
- `transform_query_normalizes_symbol_and_rejects_query_injection` —
  keeps the existing query-injection boundary active on the HTTP
  fetcher.
- `live_binance_returns_ticker_price_when_env_var_set` — additionally
  gated by `TDW_BINANCE_LIVE=1`; performs a real HTTP request to
  Binance.

See `docs/quality/production-transport-status.md` for the broader
G011 punch list.
