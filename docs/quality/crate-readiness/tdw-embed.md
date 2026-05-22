# tdw-embed Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-embed\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-embed-local, tdw-knowledge, tdw-service-api
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependencies match the embedding contract role.
- [x] Dependency direction reviewed: contract crate has no local dependencies; local providers and higher layers depend on it.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: `EmbeddingProvider`, `Embedding`, and `validate_embedding` reject empty model IDs, empty vectors, and non-finite vector values.
- [x] Runtime behavior reviewed: trait is synchronous and deterministic; it does not perform network calls.
- [x] Tests and coverage evidence recorded: contract test covers provider output plus invalid embedding shapes.
- [x] Docs and examples reviewed: this worksheet records the crate contract; no separate README/examples required for the small trait crate.
- [x] Surface wiring reviewed: tdw-embed-local, tdw-knowledge, and service-api use this abstraction.
- [x] Scaffold, dead-code, and fallback signals classified: remaining match is a test-only panic helper.
- [x] Security and reliability risks reviewed: malformed embedding payloads fail validation before storage/search layers consume them.

## Findings

- Added explicit model ID and finite-vector validation to the shared embedding contract.
- Existing local and remote adapter crates compile against the tightened contract.
- Follow-up boundary: provider-specific API transport belongs to adapter crates, not this trait crate.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are provider transport depth and production runtime integration.
