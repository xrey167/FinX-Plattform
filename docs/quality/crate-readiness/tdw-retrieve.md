# tdw-retrieve Readiness Worksheet

Owner tranche: knowledge-system overhaul B4 - Hybrid Retrieval.

## Baseline Inventory

- Manifest: crates\tdw-retrieve\Cargo.toml
- Target kinds: lib, test
- Local dependencies: tdw-core, tdw-embed, tdw-tags, tdw-taxonomy
- External dependencies: serde; serde_json
- Dev dependencies: tdw-embed-local; tdw-storage-graph; tdw-storage-meilisearch; tdw-storage-qdrant; tokio
- Reverse local dependencies: tdw-knowledge (KnowledgeIndex::search delegates here)
- Feature flags: none
- Test attributes detected: 3 (lib, pure RRF) + 5 (tests/hybrid.rs end-to-end)
- tests/ directory: yes (hybrid end-to-end over the in-memory reference engines)
- README: yes
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: none

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, publish=false, MIT OR Apache-2.0; real backends appear only as dev-dependencies (the lib codes purely against the tdw-core traits).
- [x] Dependency direction reviewed: consumes VectorEngine/LexicalEngine/GraphEngine (tdw-core), EmbeddingProvider (tdw-embed), TagEngine (tdw-tags); no storage backend leaks into the public API.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: KnowledgeQuery::try_new rejects empty text, zero or oversized top_k (MAX_TOP_K=256), malformed as_of, and out-of-range expansion (k_hop 1..=3, per_hit_limit 1..=MAX_PER_HIT_LIMIT=64, decay in (0,1]); channel failures surface the failing engine's error — never silently skipped.
- [x] Runtime behavior reviewed: tag subsumption via TagEngine::descendants; cross-channel PayloadFilter on the contract keys tags/entity_kind/plane/as_of; pure rank-only RRF (k=60, deterministic id tie-break, per-channel duplicate collapse to best rank); graph expansion is hop-bounded BFS with per-seed visited set, per_hit_limit truncation, and decay^hop contribution; final order is score desc / id asc.
- [x] Tests and coverage evidence recorded: RRF unit tests (rank-not-score fusion, exact tie-break, duplicate collapse, empty input); end-to-end tests prove multi-channel fusion with subsumption, temporal exclusion of later AND undated documents, explained graph expansion (path + seed + decayed score below seed), vector-only parity with raw KNN order, and validation rejection.
- [x] Docs and examples reviewed: crate-level docs + README document the channel model, temporal semantics, and explanation contract.
- [x] Surface wiring reviewed: tdw-knowledge::KnowledgeIndex::search delegates to a vector-only Retriever, preserving the pre-B4 order and raw-score contract (proved by the parity test).
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: forbid(unsafe_code) + inline deny(pedantic, nursery); no I/O of its own — all effects go through injected engines; expansion is bounded (k_hop <= 3, per_hit_limit, visited set) so a hostile graph cannot loop it.

## Findings

- Temporal queries are leakage-safe BY CONSTRUCTION: the as_of payload condition is a RangeString lte over normalized timestamps, so documents dated later — and documents with no date at all — are structurally invisible. The graph-derived paths (tag channel, expansion) re-apply the same contract via document-node props (document_visible: as_of/plane mirror) plus the entity node's kind, closing the 3-lens-review leak where entity_documents bypassed the payload gate. B5 ingestion stamps the props. This is the B11 eval-harness mechanism, not best-effort filtering.
- The tag channel runs only when the query is temporal (as_of present): tag activity is a dated property, and an undated tag scan would reintroduce the leakage the design removes.
- Hits explain themselves: per-channel rank + raw score, matched (subsumption-expanded) tags, and for graph-expanded hits the reaching path and seeding document. Agents can cite WHY a document surfaced (B8 read tools build on this).

## K-X3 Trust-Dial (2026-06-12)

- **`TrustClass` enum** added: `DocumentIngested` / `UserAuthored` (production-stamped), `RuleDerived` / `AgentProposed` (reserved — graph-channel only, not yet stamped on doc-index points). Rust enum is forward-compatible; MCP schema advertises only the two reachable classes today.
- **`QueryFilter::provenance_classes`** (`Option<Vec<TrustClass>>`): `None` (default) = all classes unchanged; `Some([...])` = post-gate on effective class. Old index points without the field default to `DocumentIngested` (backward-safe).
- **`RetrievedHit::trust_class`** (`Option<TrustClass>`): explainability field on every hit — `Some(class)` for directly-retrieved hits, `None` for graph-expanded stubs (effective class is `DocumentIngested` for gate purposes).
- **`assemble_hits` trust gate**: directly-retrieved candidates are filtered here; hits that fail the class gate are silently excluded (never an error).
- **Post-expansion retain pass**: after `expand_seed`, a `hits.retain` pass applies the same gate to expanded neighbors (trust_class: None → effective DocumentIngested). Without this pass a restrictive filter can be circumvented by graph expansion (K-X3 review fix #1).
- **`build_payload_filter` MatchAny pre-filter**: when the requested class set does NOT include `DocumentIngested`, a MatchAny PayloadCondition on `provenance_class` is pushed into the B1 filter so the vector/lexical channels return only matching points before fusion, avoiding top_k under-fill (K-X3 review fix #4). When `DocumentIngested` is in the set, the MatchAny is skipped so old un-stamped points (backward-compat default) are not wrongly excluded.
- **K-M3 reuse contract**: `QueryFilter::provenance_classes` is the shared filter field; `tdw.kg.answer` (K-M3) will thread the same field through at answer-synthesis time so trust-scope is consistent across search and answer. No duplication — one contract defined here.
- **Test coverage added** (13 total in tests/hybrid.rs): default-no-filter, document-only excludes user+agent, user-only returns findings, zero-match-not-error, empty-vec-same-as-none, token round-trip, and `trust_dial_expand_filter_gates_expanded_neighbors` (the expand-bypass e2e: neighbor with no stamp defaults to DocumentIngested and is excluded under a user_authored filter).

## Verification

- Focused crate check passed: `cargo test --target-dir target -p tdw-retrieve` — 13 hybrid tests pass (7 new K-X3 + 6 pre-existing).
- Lint gate passed: `cargo clippy --workspace --all-targets -- -D warnings` zero errors; `cargo fmt -p tdw-retrieve` clean.

## Verdict

Ready with follow-ups. Channels fan out sequentially (concurrent join deferred until a real-backend latency profile justifies it); B5 wires ingestion payloads, B8 exposes the retriever over MCP, B11 adds the retrieval eval harness. K-X3 trust-dial is shipped; K-M3 answer synthesis reuses the same filter contract.
