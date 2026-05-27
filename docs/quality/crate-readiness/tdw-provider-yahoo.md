# tdw-provider-yahoo Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-provider-yahoo\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core, tdw-domain, tdw-provider-fileset
- External dependencies: async-trait ^0.1.89; bytes ^1.11.0; schemars ^1.2.1; serde ^1.0.228 features=[derive]; serde_json ^1.0.145
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 4 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependencies match an offline Yahoo fetcher.
- [x] Dependency direction reviewed: depends on core/domain/fileset query validation; service-api consumes it.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: query transform delegates to fileset validation and rejects path/query unsafe symbols.
- [x] Runtime behavior reviewed: extractor returns deterministic inline Yahoo-shaped rows without live network calls or credentials.
- [x] Tests and coverage evidence recorded: tests cover registry entry, query/extract/decode flow, and unsafe symbol rejection.
- [x] Docs and examples reviewed: worksheet records the provider contract; deterministic row is inline.
- [x] Surface wiring reviewed: service-api imports the Yahoo fetcher.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: no credentials are read and query validation is shared with fileset.

## Findings

- Yahoo provider reuses the hardened fileset symbol validation boundary.
- Fetch/decode path remains deterministic and network-free for bootstrap tests.
- Follow-up boundary: real Yahoo transport, quote chart parsing, retries, and rate-limit handling belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production Yahoo transport/runtime integration.

## Production Backend Evidence (G011)

`YahooHttpEquityHistoricalFetcher` (gated by `--features http`) lives
in `crates/tdw-provider-yahoo/src/http_fetcher.rs` and implements
`tdw_core::Fetcher` against Yahoo Finance's v8 chart endpoint
directly via `reqwest`. No SDK required; Yahoo's chart API is
unauthenticated for delayed equity historical data.

Existing `YahooEquityHistoricalFetcher` keeps its synthetic
single-row behavior for offline workspace tests; the HTTP fetcher
is opt-in.

Public surface:
- `YahooHttpEquityHistoricalFetcher::default()` — base URL
  `https://query1.finance.yahoo.com`, interval `1d`, range `5d`.
- `with_base_url(url)` — point at a recorded-cassette HTTP server.
- `with_interval(interval)` / `with_range(range)` — override chart
  granularity / window (e.g. `1h` interval, `1y` range).
- `Fetcher::transform_query` reuses fileset's symbol validation /
  normalisation for consistency.
- `Fetcher::extract_data` issues `GET /v8/finance/chart/{symbol}`
  with the configured interval + range.
- `Fetcher::transform_data` parses the v8 chart envelope
  (`chart.result[0].{timestamp, indicators.quote[0]}`), filters out
  Yahoo's null bars (which appear when the requested range overlaps
  an open session), and converts Unix timestamps to `YYYY-MM-DD` via
  an inline Civil-from-Days algorithm — no chrono / time / jiff
  dependency added for this one call site.

Tests (`crates/tdw-provider-yahoo/tests/http_fetcher.rs`,
double-gated by `--features http`):
- `cassette_replay_decodes_yahoo_chart_envelope_and_skips_null_bars`
  — always runs under the feature; parses a recorded Yahoo response
  shape with three daily bars (one all-null) and asserts row
  decoding.
- `cassette_replay_surfaces_yahoo_error_envelope` — propagates
  Yahoo's `chart.error` envelope as `Error::Provider`.
- `live_yahoo_returns_recent_bars_when_env_var_set` — additionally
  gated by `TDW_YAHOO_LIVE=1`; performs a real HTTP request to
  Yahoo and asserts at least one bar comes back.

See `docs/quality/production-transport-status.md` for the broader
G011 punch list.
