# tdw-storage-graph Readiness Worksheet

Owner tranche: knowledge-system overhaul A2 - Graph Engine Contract.

## Baseline Inventory

- Manifest: crates\tdw-storage-graph\Cargo.toml
- Target kinds: lib, test
- Local dependencies: tdw-core
- External dependencies: async-trait; serde_json
- Dev dependencies: tdw-taxonomy; tokio
- Reverse local dependencies: none yet (tdw-kg facades over it in slice A3)
- Feature flags: none (the Bolt backend lands feature-gated in slice A4)
- Test attributes detected: 2 (lib) + 1 conformance suite
- tests/ directory: yes (cross-backend conformance)
- README: yes
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: none

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, publish=false, MIT OR Apache-2.0.
- [x] Dependency direction reviewed: implements the tdw-core GraphEngine trait; taxonomy only in dev-deps (kind enum flows in through tdw-core types).
- [x] Feature flags reviewed: none in A2; A4 adds the Bolt leg behind a feature + env gate mirroring the Qdrant/Postgres precedent.
- [x] Public API and error contracts reviewed: empty batches, missing endpoints, invalid ids/labels/aliases/provenance/windows, oversized hop budgets, and self-merges are explicit Error::Storage / Error::InvalidQuery rejections.
- [x] Runtime behavior reviewed: half-open [valid_from, valid_to) as_of filtering; deterministic sorted read orders; edge identity (from, to, rel, valid_from) upsert-replacement; BFS expansion and shortest path are hop-bounded by MAX_HOPS.
- [x] Tests and coverage evidence recorded: conformance suite covers round-trips, direction/rel/kind/as_of filters, hop-bounded expand, shortest path incl. budget and unreachability, and full merge semantics (alias union, rewiring with duplicate/self-loop dropping, tombstone, Manual-provenance audit edge).
- [x] Docs and examples reviewed: README documents the contract highlights and the A4 conformance plan.
- [x] Surface wiring reviewed: no production consumers yet by design; tdw-kg facades over GraphEngine in slice A3.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: forbid(unsafe_code); validated constructors reject control characters and traversal-shaped ids; mutex-poisoning maps to Error::Storage rather than panicking.

## Findings

- In-memory reference engine is the deterministic conformance baseline; the dedicated graph database (Memgraph via Bolt) joins the same suite in A4.
- Merge is a real merge (rewiring + tombstone + audit), replacing tdw-kg's audit-only manual_merge from slice A3 onward.

## Verification

- Focused crate check passed: cargo test --target-dir target -p tdw-core -p tdw-storage-graph.
- Lint gate passed: cargo fmt -p tdw-core -p tdw-storage-graph -- --check; cargo clippy --target-dir target -p tdw-core -p tdw-storage-graph --all-targets (pedantic+nursery via workspace lints).

## Verdict

Ready with follow-ups. The Bolt backend and its conformance leg land in knowledge-system slice A4; tdw-kg adoption lands in A3.
