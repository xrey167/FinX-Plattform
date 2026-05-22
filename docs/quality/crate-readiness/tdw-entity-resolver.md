# tdw-entity-resolver Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-entity-resolver\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-kg
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
- [x] Dependency direction reviewed.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: compatibility resolve/merge APIs remain and checked try_* APIs expose invalid symbol/merge endpoint errors.
- [x] Runtime behavior reviewed: symbol resolution rejects path/query-like symbols before matching aliases.
- [x] Tests and coverage evidence recorded: tests cover exact matching and unsafe symbol/self-merge rejection.
- [x] Docs and examples reviewed: worksheet records resolver behavior; no README/examples required.
- [x] Surface wiring reviewed: service API uses resolver with valid symbols.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: untrusted symbols cannot traverse into graph IDs or merge endpoints unchecked.

## Findings

- Resolver remains deterministic and in-memory over provided KG entities.
- Checked APIs now make malformed resolution and merge requests auditable.
- Follow-up boundary: fuzzy scoring and conflict review queues belong in later resolver runtime work.

## Verification

- Focused G006 crate check passed: cargo test -p tdw-entity-resolver -p tdw-eval-runner -p tdw-feature-store -p tdw-fn-string -p tdw-graph -p tdw-kg -p tdw-knowledge -p tdw-ml-registry -p tdw-rewrite -p tdw-spatial -p tdw-tag-rules -p tdw-tags -p tdw-test-utils -p tdw-workflow-engine -p tdw-service-api.
- Final workspace gate for G006 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G006 blocker remains inside tdw-entity-resolver.
