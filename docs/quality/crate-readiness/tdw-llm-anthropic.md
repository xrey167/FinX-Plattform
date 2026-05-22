# tdw-llm-anthropic Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-llm-anthropic\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-llm
- External dependencies: none
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependency shape match a model adapter.
- [x] Dependency direction reviewed: depends only on tdw-llm; service-api consumes it.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: constructor validates model ID and completion uses shared chat-request validation.
- [x] Runtime behavior reviewed: adapter is a deterministic offline model stub and performs no live Anthropic network call.
- [x] Tests and coverage evidence recorded: test covers valid completion, invalid model ID, control-character model IDs, and invalid prompt content.
- [x] Docs and examples reviewed: no separate README/examples required for this offline adapter contract.
- [x] Surface wiring reviewed: service-api imports the Anthropic adapter.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: invalid prompts and model identifiers fail through the shared contract.

## Findings

- Anthropic adapter remains deterministic and transport-free while preserving provider-specific model output labeling.
- New validation evidence proves it does not bypass the shared chat/model checks.
- Follow-up boundary: real Anthropic Messages API HTTP transport, credentials, and streaming belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production Anthropic transport/runtime integration.
