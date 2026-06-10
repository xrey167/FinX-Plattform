# tdw-taxonomy Readiness Worksheet

Owner tranche: knowledge-system overhaul A1 - Unified Entity Taxonomy.

## Baseline Inventory

- Manifest: crates\tdw-taxonomy\Cargo.toml
- Target kinds: lib
- Local dependencies: none
- External dependencies: schemars; serde features=[derive]; validator
- Dev dependencies: serde_json
- Reverse local dependencies: tdw-agent
- Feature flags: none
- Test attributes detected: 10
- tests/ directory: no
- README: yes
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: none

## Required Readiness Evidence

- [x] Manifest correctness reviewed: leaf crate, workspace lints, publish=false, MIT OR Apache-2.0.
- [x] Dependency direction reviewed: pure-data leaf crate; tdw-agent re-exports it from the original module paths so all pre-A1 consumers compile unchanged.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: EntityKind (50 kinds, 8 manifest groups incl. the new domain group), facets (DataFacets/EvalFacets with validator constraints), Origin (tier x source); types moved verbatim from tdw-agent.
- [x] Runtime behavior reviewed: no I/O; declaration order documented as load-bearing (Ord drives ALL and Registry iteration) and the domain group is appended after meta to keep the 43 pre-existing ordinals stable.
- [x] Tests and coverage evidence recorded: kind uniqueness/count, per-group decision-table counts, lowercase serde token round-trips (old and new kinds), autonomy capability, is_data_kind/Group::Knowledge equivalence pin, ordinal-stability pin, facets round-trips, Origin round-trip.
- [x] Docs and examples reviewed: crate README documents scope, design constraints, and the re-export compatibility contract.
- [x] Surface wiring reviewed: tdw-agent shims (kind, facets, base::{Origin,Source,Tier}) re-export the moved types; tdw-backend and tdw-mcp test suites green against the new layout.
- [x] Scaffold, dead-code, and fallback signals classified: none; the 7 domain kinds are deliberate candidate kinds whose spec types land in slice A3 (spec_schema_for returns None for them, matching the documented ResourceDefinition contract).
- [x] Security and reliability risks reviewed: pure data types, forbid(unsafe_code), no new dependency surface beyond serde/schemars/validator already in the workspace.

## Findings

- Single source of truth for entity classification shared by the agent and warehouse planes; first slice of unifying the 5-kind tdw-kg enum with the platform taxonomy (completed in A3).
- Domain kinds are candidate kinds: classified and serializable now, concrete spec types deferred to A3.

## Verification

- Focused crate check passed: cargo test --target-dir target -p tdw-taxonomy -p tdw-agent -p tdw-backend -p tdw-mcp.
- Lint gate passed: cargo fmt --all -- --check; cargo clippy --target-dir target -p tdw-taxonomy -p tdw-agent -p tdw-backend -p tdw-mcp --all-targets (pedantic+nursery, -D warnings via workspace lints).

## Verdict

Ready with follow-ups. Domain-kind spec types and the tdw-kg EntityKind unification land in knowledge-system slice A3.
