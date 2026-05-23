# tdw-spatial Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-spatial\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: standalone spatial primitive.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: point, bounding box, contains, distance, and validation errors are explicit.
- [x] Runtime behavior reviewed: checked paths reject NaN/Inf coordinates, out-of-range lat/lon, and inverted boxes.
- [x] Tests and coverage evidence recorded: tests cover contains/distance and invalid coordinate/box rejection.
- [x] Docs and examples reviewed: worksheet records spatial behavior.
- [x] Surface wiring reviewed: service API spatial sample remains green.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: non-finite spatial inputs are rejected on checked paths.

## Findings

- Spatial remains a small bootstrap geometry helper.
- Checked APIs now prevent invalid coordinate propagation.

## Verification

- Focused G006 crate check passed: cargo test -p tdw-entity-resolver -p tdw-eval-runner -p tdw-feature-store -p tdw-fn-string -p tdw-graph -p tdw-kg -p tdw-knowledge -p tdw-ml-registry -p tdw-rewrite -p tdw-spatial -p tdw-tag-rules -p tdw-tags -p tdw-test-utils -p tdw-workflow-engine -p tdw-service-api.
- Final workspace gate for G006 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G006 blocker remains inside tdw-spatial.
