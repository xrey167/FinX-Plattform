# tdw-llm Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-llm\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-config
- External dependencies: serde ^1.0.228 features=[derive]; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-llm-anthropic, tdw-llm-openai-compat, tdw-service-api
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependencies match the shared model contract.
- [x] Dependency direction reviewed: depends on tdw-config for model selection; adapter crates and service-api consume the contract.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: chat requests reject empty messages/content and zero max tokens; model/base URL validators reject empty, control, unsafe, or unsupported values.
- [x] Runtime behavior reviewed: language model trait is deterministic and transport-free in this contract crate.
- [x] Tests and coverage evidence recorded: model trait test covers completion, last-user selection, invalid token budget, unsafe model IDs, and unsafe base URLs.
- [x] Docs and examples reviewed: no separate README/examples required for this shared contract.
- [x] Surface wiring reviewed: Anthropic/OpenAI-compatible adapters and service-api depend on this crate.
- [x] Scaffold, dead-code, and fallback signals classified: remaining match is a test-only panic helper.
- [x] Security and reliability risks reviewed: unsafe model IDs/URLs and invalid chat payloads fail before adapter execution.

## Findings

- Tightened model ID and base URL validation to reject control/whitespace injection paths.
- `last_user_message` validates the whole request before returning prompt content.
- Follow-up boundary: provider-specific auth, streaming, and HTTP execution belong to concrete model adapters.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are provider transport and streaming depth.
