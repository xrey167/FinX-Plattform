# tdw-embed-local Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-embed-local\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-embed
- External dependencies: none
- Dev dependencies: none
- Reverse local dependencies: tdw-knowledge, tdw-service-api
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependency shape match the local embedding adapter role.
- [x] Dependency direction reviewed: depends only on tdw-embed; knowledge and service-api consume it.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: constructor rejects empty model IDs and zero dimensions.
- [x] Runtime behavior reviewed: hash embedding is deterministic, local-only, and does not require credentials or network.
- [x] Tests and coverage evidence recorded: deterministic embedding test covers repeatability, vector length, validation, and bad constructor inputs.
- [x] Docs and examples reviewed: no separate README/examples required for this small local adapter.
- [x] Surface wiring reviewed: tdw-knowledge/service-api use the local adapter as the offline embedding path.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: invalid model/dimension configuration fails before embeddings are generated.

## Findings

- Local hash embeddings are deterministic and validate through the shared embedding contract.
- Constructor now rejects empty model IDs instead of silently creating an unnamed provider.
- Follow-up boundary: production-grade local model inference can replace the hash adapter behind the same trait.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production embedding runtime depth.
