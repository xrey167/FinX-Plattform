# tdw-knowledge Readiness Worksheet

Owner tranche: G006-knowledge-graph-tags-ml-eval-and-utility-crates - Knowledge, Graph, Tags, ML, Eval, and Utility Crates.

## Baseline Inventory

- Manifest: crates\tdw-knowledge\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core, tdw-embed, tdw-embed-local, tdw-kg, tdw-storage-qdrant, tdw-tags
- External dependencies: serde ^1.0.228 features=[derive]; serde_json ^1.0.145; thiserror ^2.0.18; tokio ^1.52.3 features=[macros, rt]
- Dev dependencies: none
- Reverse local dependencies: tdw-service-api
- Feature flags: none
- Test attributes detected: 4
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 4 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: knowledge index composes embed, KG, vector storage, and tags.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: document, query, payload, embedding, storage, and tag errors are explicit.
- [x] Runtime behavior reviewed: index/search reject malformed document IDs, empty bodies, bad tags, blank queries, and zero top_k.
- [x] Tests and coverage evidence recorded: tests cover indexing/search, malformed payloads, invalid documents/queries, and syntax summary.
- [x] Docs and examples reviewed: worksheet records knowledge behavior.
- [x] Surface wiring reviewed: service API knowledge sample remains green.
- [x] Scaffold, dead-code, and fallback signals classified: remaining scan signals are test-only panic helpers.
- [x] Security and reliability risks reviewed: malformed documents/queries do not reach vector/KG/tag stores on checked paths.

## Findings

- KnowledgeIndex is a composed in-memory bootstrap index.
- Payload validation and document/query validation now cover the primary corruption paths.

## Verification

- Focused G006 crate check passed: cargo test -p tdw-entity-resolver -p tdw-eval-runner -p tdw-feature-store -p tdw-fn-string -p tdw-graph -p tdw-kg -p tdw-knowledge -p tdw-ml-registry -p tdw-rewrite -p tdw-spatial -p tdw-tag-rules -p tdw-tags -p tdw-test-utils -p tdw-workflow-engine -p tdw-service-api.
- Final workspace gate for G006 passed: cargo fmt --all -- --check; cargo check --workspace; cargo clippy --workspace --all-targets -- -D warnings; cargo test --workspace; cargo run -p xtask -- clean-room-audit; git diff --check.

## Verdict

Ready with follow-ups. No G006 blocker remains inside tdw-knowledge.
