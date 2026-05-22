# tdw-test-utils Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-test-utils\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-domain
- External dependencies: none
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: default; e2e; integration; property
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: test utility crate depends only on tdw-domain.
- [x] Feature flags reviewed: default/e2e/integration/property are declared as empty coordination flags.
- [x] Public API and error contracts reviewed: fixtures and container specs are deterministic bootstrap helpers.
- [x] Runtime behavior reviewed: fixture data is stable and container specs cover the minimal profile.
- [x] Tests and coverage evidence recorded: tests cover deterministic OHLCV fixtures and core container specs.
- [x] Docs and examples reviewed: worksheet records test utility behavior.
- [x] Surface wiring reviewed: no runtime consumer.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: no network or container launch happens in this crate.

## Findings

- This crate remains fixture-only and does not start infrastructure.
- Feature flags are coordination markers for future test layers.

## Verification

- Focused G006 crate check passed: cargo test -p tdw-entity-resolver -p tdw-eval-runner -p tdw-feature-store -p tdw-fn-string -p tdw-graph -p tdw-kg -p tdw-knowledge -p tdw-ml-registry -p tdw-rewrite -p tdw-spatial -p tdw-tag-rules -p tdw-tags -p tdw-test-utils -p tdw-workflow-engine -p tdw-service-api.
- Final workspace gate for G006 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G006 blocker remains inside tdw-test-utils.
