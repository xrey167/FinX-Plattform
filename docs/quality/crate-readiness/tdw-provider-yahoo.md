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
