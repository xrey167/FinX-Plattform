# tdw-provider-fred Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-provider-fred\Cargo.toml
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
- [x] Public API and error contracts reviewed: request builder rejects missing API key, empty series IDs, and query-unsafe series IDs.
- [x] Runtime behavior reviewed: builds typed FRED series-observations request metadata without performing live network calls or storing secrets.
- [x] Tests and coverage evidence recorded: test covers endpoint metadata, credential parameter, normalization, missing key, empty series, and query injection rejection.
- [x] Docs and examples reviewed: worksheet records the provider contract; no separate README/examples required.
- [x] Surface wiring reviewed: no higher-level crate currently depends directly on this provider.
- [x] Scaffold, dead-code, and fallback signals classified: former stub signal removed; remaining match is a test-only panic helper.
- [x] Security and reliability risks reviewed: API key handling is presence-only and series ID input cannot add query parameters.

## Findings

- FRED provider is an offline request-contract crate, not a live data client.
- Series ID validation constrains query composition while preserving common FRED ID characters.
- Follow-up boundary: HTTP execution, pagination, FRED error decoding, and secret loading belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production FRED transport/runtime integration.

## Production Backend Evidence (G011)

`FredHttpSeriesObservationsFetcher` (gated by `--features http`) lives
in `crates/tdw-provider-fred/src/http_fetcher.rs` and implements
`tdw_core::Fetcher` against FRED's `/series/observations` endpoint
directly via `reqwest`. No SDK required; live calls load the API key
from `FRED_API_KEY`.

Existing `series_observations_request` keeps the request-contract
surface and series ID validation for offline tests and downstream
callers.

Public surface:
- `FredSeriesObservationsQuery::new(series_id)` — validates and
  normalizes FRED series IDs.
- `FredObservation` — decoded row shape with `series_id`, `date`,
  `value`, `realtime_start`, and `realtime_end`.
- `FredHttpSeriesObservationsFetcher::default()` — base URL
  `https://api.stlouisfed.org/fred`, bounded by a default
  `limit=1000`.
- `with_base_url(url)` — point at an alternate FRED-compatible
  endpoint.
- `with_observation_start(date)` / `with_observation_end(date)` /
  `with_limit(limit)` — constrain the remote observation window.
- `Fetcher::transform_query` accepts `{ "series_id": "GDP" }` and
  reuses the existing series ID validation boundary.
- `Fetcher::extract_data` issues `GET /series/observations` with
  `series_id`, `api_key`, `file_type=json`, and optional date/limit
  query parameters.
- `Fetcher::transform_data` parses FRED's JSON observations envelope,
  skips missing-value `"."` observations, and propagates FRED
  `error_code` / `error_message` envelopes as `Error::Provider`.

Tests (`crates/tdw-provider-fred/tests/http_fetcher.rs`, double-gated
by `--features http`):
- `cassette_replay_decodes_observations_and_skips_missing_values` —
  always runs under the feature; parses a recorded FRED response shape
  and asserts row decoding.
- `cassette_replay_surfaces_fred_error_envelope` — propagates FRED's
  JSON error envelope as `Error::Provider`.
- `transform_query_normalizes_series_id_and_rejects_query_injection`
  — keeps the existing query-injection boundary active on the HTTP
  fetcher.
- `live_fred_returns_recent_observations_when_env_vars_set` —
  additionally gated by `TDW_FRED_LIVE=1`; requires `FRED_API_KEY` and
  performs a real HTTP request to FRED.

See `docs/quality/production-transport-status.md` for the broader
G011 punch list.
