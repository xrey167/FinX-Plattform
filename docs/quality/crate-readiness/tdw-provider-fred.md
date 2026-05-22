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
