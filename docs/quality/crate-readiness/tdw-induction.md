# tdw-induction Readiness Worksheet

Owner tranche: K-R5 - Pattern-to-rule induction through the walk-forward replay gate.

## Baseline Inventory

- Manifest: crates\tdw-induction\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-core, tdw-eval-runner, tdw-infer, tdw-patterns
- External dependencies: serde, serde_json, thiserror (workspace)
- Dev dependencies: tokio, tdw-storage-graph
- Reverse local dependencies: tdw-backend
- Feature flags: none
- Test attributes detected: 20
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 0 total

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed: induction is a domain-layer crate consumed by backend.
- [x] Feature flags reviewed: none.
- [x] Public API and error contracts reviewed: InductionEngine, rule proposal routing, and replay gate wiring checked.
- [x] Runtime behavior reviewed: induction sweep spawned by backend serve(), shutdown cleans up task handle.
- [x] Tests and coverage evidence recorded: unit tests cover induction gate logic and proposal routing.
- [x] Docs and examples reviewed: worksheet records induction-engine behavior.
- [x] Surface wiring reviewed: tdw-backend spawns induction task; rules routed through walk-forward replay gate (K-R7) before promotion to proposals.
- [x] Scaffold, dead-code, and fallback signals classified: none detected.
- [x] Security and reliability risks reviewed: induction worker is cancellation-safe via JoinHandle shutdown; fail-closed on gate errors.

## Findings

- K-R5 adds rule induction: patterns promoted to infer rules via the K-R7 walk-forward replay gate.
- InductionEngine runs as a background sweep worker inside the daemon.
- Rules are self-authored and routed as proposals for operator review (no direct materialization).
- No external network or credential access required for induction.

## Verification

- Focused K-R5 crate check passed: cargo test -p tdw-induction -p tdw-infer -p tdw-backend --no-default-features.
- Final workspace gate for K-R5 passed: cargo check --workspace --all-targets.

## Verdict

Ready with follow-ups. No K-R5 blocker remains inside tdw-induction.
