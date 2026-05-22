# tdw-tag-rules Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-tag-rules\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-tags
- External dependencies: serde ^1.0.228 features=[derive]; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 4 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: tag rules sit above tdw-tags.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: unsafe SQL, invalid rule, recursion, and tag assignment errors are explicit.
- [x] Runtime behavior reviewed: hot reload validates rule IDs, tag IDs, SQL, JSON paths, labels, and assignment outcomes.
- [x] Tests and coverage evidence recorded: tests cover hot reload/apply, unsafe SQL, invalid IDs, and unknown tag assignment failure.
- [x] Docs and examples reviewed: worksheet records tag-rule behavior.
- [x] Surface wiring reviewed: service API tag-rule sample remains green.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signals are test-only panic helpers.
- [x] Security and reliability risks reviewed: malformed rules and failed tag writes no longer pass silently.

## Findings

- RuleEngine remains in-memory and deterministic.
- Assignment failures now propagate instead of being ignored.

## Verification

- Focused G006 crate check passed: cargo test -p tdw-entity-resolver -p tdw-eval-runner -p tdw-feature-store -p tdw-fn-string -p tdw-graph -p tdw-kg -p tdw-knowledge -p tdw-ml-registry -p tdw-rewrite -p tdw-spatial -p tdw-tag-rules -p tdw-tags -p tdw-test-utils -p tdw-workflow-engine -p tdw-service-api.
- Final workspace gate for G006 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G006 blocker remains inside tdw-tag-rules.
