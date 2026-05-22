# tdw-workflow-engine Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-workflow-engine\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-agent
- External dependencies: serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: workflow engine depends on agent workflow contracts.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: compile now uses full agent workflow contract validation, not DAG-only validation.
- [x] Runtime behavior reviewed: invalid workflow IDs are rejected before execution plans are emitted.
- [x] Tests and coverage evidence recorded: tests cover valid plan compilation and invalid workflow ID rejection.
- [x] Docs and examples reviewed: worksheet records workflow engine behavior.
- [x] Surface wiring reviewed: service API workflow sample remains green.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signal is a test-only panic helper.
- [x] Security and reliability risks reviewed: checked compilation prevents malformed workflow identifiers from entering execution plans.

## Findings

- WorkflowEngine remains a deterministic DAG compiler.
- It now honors the stricter agent workflow contract added in G005.

## Verification

- Focused G006 crate check passed: cargo test -p tdw-entity-resolver -p tdw-eval-runner -p tdw-feature-store -p tdw-fn-string -p tdw-graph -p tdw-kg -p tdw-knowledge -p tdw-ml-registry -p tdw-rewrite -p tdw-spatial -p tdw-tag-rules -p tdw-tags -p tdw-test-utils -p tdw-workflow-engine -p tdw-service-api.
- Final workspace gate for G006 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G006 blocker remains inside tdw-workflow-engine.
