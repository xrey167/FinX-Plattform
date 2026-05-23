# tdw-provider-ws-mock Readiness Worksheet

Owner tranche: G004-provider-embedding-and-model-adapter-crates - Provider, Embedding, and Model Adapter Crates.

## Baseline Inventory

- Manifest: crates\tdw-provider-ws-mock\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core, tdw-domain
- External dependencies: async-trait ^0.1.89; futures-core ^0.3.31; schemars ^1.2.1; serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 6 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, edition 2024, publish=false, and dependencies match a deterministic mock streamer.
- [x] Dependency direction reviewed: depends on core/domain only; service-api consumes it.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: snapshot and subscribe reject empty and unsupported symbols while trimming and uppercasing valid symbols.
- [x] Runtime behavior reviewed: snapshot and stream return deterministic in-memory market data and require no credentials/network.
- [x] Tests and coverage evidence recorded: tests cover registry entry, snapshot/stream parity, end-of-stream behavior, empty symbols, and unsafe symbols.
- [x] Docs and examples reviewed: worksheet records the mock provider contract; deterministic row is inline.
- [x] Surface wiring reviewed: service-api imports the mock streamer for aggregate samples.
- [x] Scaffold, dead-code, and fallback signals classified: remaining matches are test-only panic helpers.
- [x] Security and reliability risks reviewed: mock streamer cannot leak credentials and rejects path/query unsafe symbols.

## Findings

- Mock streamer now normalizes symbol input before both snapshot and stream paths.
- Stream implementation remains deterministic and immediately ready for tests.
- Follow-up boundary: live websocket reconnection/backpressure belongs to real provider/runtime work, not this mock.

## Verification

- Focused G004 crate check passed: cargo test -p tdw-provider-alpaca -p tdw-provider-binance -p tdw-provider-fileset -p tdw-provider-fred -p tdw-provider-huggingface -p tdw-provider-polygon -p tdw-provider-ws-mock -p tdw-provider-yahoo -p tdw-embed -p tdw-embed-local -p tdw-embed-openai -p tdw-embed-google -p tdw-llm -p tdw-llm-anthropic -p tdw-llm-openai-compat.
- Final workspace gate for G004: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G004 blocker remains; follow-ups are live websocket provider/runtime integration.
