# tdw-embed-google Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-embed-google\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-embed
- External dependencies: serde_json ^1.0.145; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 1
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 2 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependencies match an offline HTTP request adapter.
- [x] Dependency direction reviewed: depends on the embedding contract only.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: request builder rejects missing API key, empty model, and empty input; decoder rejects empty and non-finite vectors.
- [x] Runtime behavior reviewed: builds Google embedContent request payloads without live network calls or secret material.
- [x] Tests and coverage evidence recorded: request/decoder test covers credential requirement and invalid payload boundaries.
- [x] Docs and examples reviewed: worksheet records the adapter contract; no separate README/examples required.
- [x] Surface wiring reviewed: no higher-level crate currently depends directly on this adapter.
- [x] Scaffold, dead-code, and fallback signals classified: former stub signal removed; remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: API keys are represented only as caller-supplied presence flags and are never stored.

## Findings

- Google embedding adapter is an offline request/response contract, not a live client.
- Decode path now rejects non-finite vectors in addition to empty vectors.
- Follow-up boundary: HTTP execution, retries, rate limits, and secret loading belong to runtime/provider integration.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are production Google transport/runtime integration.
