# tdw-tags Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-tags\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]; thiserror ^2.0.18
- Dev dependencies: none
- Reverse local dependencies: tdw-feature-store, tdw-knowledge, tdw-service-api, tdw-tag-rules
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 4 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: foundational tag store with expected reverse consumers.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: invalid IDs, unknown parents/tags, cycles, and invalid assignments are explicit.
- [x] Runtime behavior reviewed: definitions validate tag IDs/parents/TTL and assignments validate entity/date/provenance/expiry.
- [x] Tests and coverage evidence recorded: tests cover DAG/TTL/provenance/stats and invalid taxonomy/assignment rejection.
- [x] Docs and examples reviewed: worksheet records tag-store behavior.
- [x] Surface wiring reviewed: feature, knowledge, tag-rule, and service API samples remain green.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signals are test-only panic helpers.
- [x] Security and reliability risks reviewed: malformed tags and invalid temporal assignments cannot enter checked store paths.

## Findings

- TagStore remains an in-memory taxonomy and assignment store.
- Parent existence and expiry ordering are now enforced.

## Verification

- Focused G006 crate check passed: cargo test -p tdw-entity-resolver -p tdw-eval-runner -p tdw-feature-store -p tdw-fn-string -p tdw-graph -p tdw-kg -p tdw-knowledge -p tdw-ml-registry -p tdw-rewrite -p tdw-spatial -p tdw-tag-rules -p tdw-tags -p tdw-test-utils -p tdw-workflow-engine -p tdw-service-api.
- Final workspace gate for G006 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G006 blocker remains inside tdw-tags.
