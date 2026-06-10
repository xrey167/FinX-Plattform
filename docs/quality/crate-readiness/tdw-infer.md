# tdw-infer Readiness Worksheet

Owner tranche: knowledge-system overhaul B7 - Rule-based Inference.

## Baseline Inventory

- Manifest: crates\tdw-infer\Cargo.toml
- Target kinds: lib, test
- Local dependencies: tdw-core, tdw-tags
- External dependencies: serde; serde_json; thiserror
- Dev dependencies: tdw-storage-graph; tdw-taxonomy; tokio
- Reverse local dependencies: none yet (B8 MCP exposure and the daemon wire it later)
- Feature flags: none
- Test attributes detected: 8 (lib: rule validation, stratification, key parsing, index round-trip) + 7 (tests/infer.rs end-to-end)
- tests/ directory: yes (forward-chaining end-to-end over the in-memory reference engines)
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: none

## Required Readiness Evidence

- [x] Manifest correctness reviewed: workspace lints, publish=false, MIT OR Apache-2.0; the graph/tag reference engines appear only as dev-dependencies (the lib codes purely against the tdw-core `GraphEngine` and tdw-tags `TagEngine` traits).
- [x] Dependency direction reviewed: consumes `GraphEngine`/`Provenance`/`TraversalFilter` (tdw-core) and `TagEngine`/`TagAssignment` (tdw-tags); no storage backend leaks into the public API.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: `InferEngine::hot_reload` validates EVERY rule (shape + grammar + chain length 1..=3 + hop bounds 1..=4 + no self-recursion) AND set-level stratification (a consumed derived type must come from a STRICTLY LOWER stratum, else `InferError::Unstratifiable` naming both rule ids) or changes nothing and never bumps the version; `RunLimits` (default 32 iterations / 10_000 derived) exceedance is `InferError::IterationLimitExceeded` / `DerivedLimitExceeded`, never silent truncation; engine/serde failures surface as `InferError::{Graph,Tag,Serde}`.
- [x] Runtime behavior reviewed: `DeriveEdge` chains are matched by paging `GraphEngine::edges(Some(rel), offset, 256)` and joining head-to-tail in memory (endpoints = first `from`, last `to`), self-loops skipped, dedup via the derivation index, written with `Provenance::Rule{rule_id, version}`; `PropagateTag` seeds from `entities_with_tag` (plus `descendants` under subsumption), walks `along` edges via `expand` in the chosen direction up to `max_hops`, assigns the BASE tag with provenance `derived:rule:<id>@v<version>` at the injected `now`, skipping already-active tags; `run_full` processes strata ascending, iterating each to a no-new-facts fixpoint; `run_incremental` fires only rules whose inputs intersect the `ChangeSet`; `retract` deletes derived edges whose support transitively includes the retracted fact via `delete_edges`.
- [x] Tests and coverage evidence recorded: chain derivation with Rule provenance + idempotent re-run + self-loop skip; two-hop stratified cascade + same-stratum rejection + self-recursion rejection; tag propagation with descendant subsumption + already-active skip + provenance stamp; both limits as errors; `run_incremental` fires only on intersecting change sets; `retract` cascades edge deletion transitively and reports unremovable tags; `DerivationIndex` to_json/from_json round-trip.
- [x] Docs and examples reviewed: crate-level docs document the rule forms, termination argument, the honest "full re-scan, NOT semi-naive" disclosure, the incremental under-approximation, and the append-only tag retraction limitation.
- [x] Surface wiring reviewed: no consumer yet; B8 exposes the engine over MCP and the daemon schedules runs.
- [x] Scaffold, dead-code, and fallback signals classified: none.
- [x] Security and reliability risks reviewed: forbid(unsafe_code) + inline deny(pedantic, nursery); no I/O of its own — all effects go through injected engines, persistence of the `DerivationIndex` is the caller's concern (to_json/from_json, no filesystem access); termination is bounded by stratification + monotonicity AND by `RunLimits` as defense-in-depth, so a pathological rule set cannot loop the fixpoint.

## Findings

- Termination is guaranteed two ways: stratified monotone rules reach a fixpoint over the finite fact universe, and `RunLimits` is a hard ceiling whose exceedance is a loud error. This is the B7 "exceeding is an ERROR, not silent truncation" contract.
- Honest about its v1 shape: `run_full` is a FULL re-scan per iteration with a no-new-facts check — it is correct and terminating but is NOT semi-naive, and the docs say so rather than overclaiming.
- Retraction is deliberately partial: derived EDGES are deleted transitively by support closure, but `tdw-tags` is append-only with no unassign primitive, so derived TAGS are reported in `RetractReport::unremovable_tags` and the documented fallback is a full re-run from a clean graph (matches plan risk R7).

## Verification

- Focused crate check passed: cargo test --target-dir target -p tdw-infer (8 lib + 7 integration).
- Lint gate passed: cargo fmt -p tdw-infer -- --check; cargo clippy --target-dir target -p tdw-infer --all-targets (pedantic+nursery, inline deny, zero warnings).

## Verdict

Ready with follow-ups. The engine is complete against the in-memory reference backends; B8 wires it over MCP and the daemon schedules incremental runs; true truth-maintenance (full retraction including tags) is deferred per plan risk R7.
