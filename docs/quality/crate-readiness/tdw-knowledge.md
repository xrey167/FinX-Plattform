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

## K-X3 Trust-Dial (2026-06-12)

- **`document_payload` stamps `provenance_class`**: the `provenance_class_token(kind)` helper maps `EntityKind::Finding` → `"user_authored"` and all other kinds → `"document_ingested"`. This stamp appears on every vector point upserted by `index_at`.
- **Production-path coverage**: `KnowledgeIndexer::index_at` calls `document_payload` for both the vector point and the lexical co-index payload, so both retrieval channels see the stamp. The graph stamping path (`write_durable_graph`) is unchanged — graph-channel provenance is tracked via `Provenance` variants (not the `provenance_class` payload field, which is doc-index only).
- **Honest class assignment**: `RuleDerived` and `AgentProposed` tokens are NOT written by `document_payload` because graph-derived facts write to the graph engine, not to doc-index points. These classes are reserved for a future graph-channel filter path (K-M3 or later). The MCP schema correctly advertises only `document_ingested` and `user_authored` as reachable today.

## Verification

- Focused crate check passed: `cargo test --target-dir target -p tdw-knowledge` — all tests pass including the production-path stamp assertion in `crates/tdw-mcp/tests/knowledge_tools.rs::index_at_stamps_provenance_class_on_production_path`.
- Lint gate passed: `cargo clippy --workspace --all-targets -- -D warnings` zero errors; `cargo fmt -p tdw-knowledge` clean.

## Verdict

Ready with follow-ups. No blocker remains inside tdw-knowledge. K-X3 provenance stamping is wired through `document_payload`; trust-class filtering is handled downstream in tdw-retrieve.
