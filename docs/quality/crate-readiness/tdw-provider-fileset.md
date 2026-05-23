# tdw-provider-fileset Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-provider-fileset\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core, tdw-domain
- External dependencies: async-trait ^0.1.89; bytes ^1.11.0; schemars ^1.2.1; serde ^1.0.228 features=[derive]; serde_json ^1.0.145
- Dev dependencies: none
- Reverse local dependencies: tdw-provider-yahoo, tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependencies match an offline fetcher contract.
- [x] Dependency direction reviewed: depends on core/domain only; Yahoo and service-api consume it.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: query transform rejects missing, empty, and unsupported symbols while trimming and uppercasing valid symbols.
- [x] Runtime behavior reviewed: extract/transform uses deterministic in-memory fixture rows and performs no network calls or credential reads.
- [x] Tests and coverage evidence recorded: tests cover registry entry, empty/unsafe symbols, and whitespace normalization.
- [x] Docs and examples reviewed: worksheet records the provider contract; fixture data is inline and deterministic.
- [x] Surface wiring reviewed: Yahoo delegates query validation and service-api imports the provider.
- [x] Scaffold, dead-code, and fallback signals classified: remaining match is a test-only panic helper.
- [x] Security and reliability risks reviewed: fixture provider cannot leak credentials and rejects path/query unsafe symbols.

## Findings

- Fileset provider now trims before uppercasing symbols, fixing whitespace-preserving query normalization.
- Fixture extraction remains deterministic and schema-typed through tdw-domain.
- Follow-up boundary: real filesystem dataset discovery and configurable fixture roots belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production fileset discovery/runtime integration.

## Smoke Evidence (G009)

Participates in the [end-to-end functional smoke](../end-to-end-smoke.md). The smoke composition is exercised by:

- `tdw-test-utils::smoke::run_end_to_end_smoke` (library entry)
- `crates/tdw-test-utils/tests/end_to_end_smoke.rs` (integration tests)
- `tdw-service` and `tdw-cli` binaries (programmatic harness output)

Verified with `cargo test -p tdw-test-utils --test end_to_end_smoke` — green.
