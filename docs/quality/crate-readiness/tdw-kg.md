# tdw-kg Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-kg\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: serde ^1.0.228 features=[derive]
- Dev dependencies: none
- Reverse local dependencies: tdw-entity-resolver, tdw-knowledge, tdw-service-api
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 1 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: KG is a lower-level graph/entity contract.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: checked entity and relationship insertion validates IDs, labels, aliases, endpoints, and provenance.
- [x] Runtime behavior reviewed: manual merge refuses missing endpoints and empty approvers.
- [x] Tests and coverage evidence recorded: tests cover normal graph use and invalid entity/missing edge rejection.
- [x] Docs and examples reviewed: worksheet records KG behavior.
- [x] Surface wiring reviewed: resolver, knowledge, and service API samples remain green.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signal is a test-only panic helper.
- [x] Security and reliability risks reviewed: checked graph mutation prevents malformed IDs and dangling relationships.

## Findings

- KnowledgeGraph remains in-memory bootstrap storage.
- Checked APIs are now available for untrusted entity and relationship ingestion.

## Verification

- Focused G006 crate check passed: cargo test -p tdw-entity-resolver -p tdw-eval-runner -p tdw-feature-store -p tdw-fn-string -p tdw-graph -p tdw-kg -p tdw-knowledge -p tdw-ml-registry -p tdw-rewrite -p tdw-spatial -p tdw-tag-rules -p tdw-tags -p tdw-test-utils -p tdw-workflow-engine -p tdw-service-api.
- Final workspace gate for G006 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G006 blocker remains inside tdw-kg.
