# xtask Readiness Worksheet

Owner tranche: G008-aggregate-production-readiness-gate - Aggregate Production Readiness Gate.

## Baseline Inventory

- Manifest: xtask\Cargo.toml
- Target kinds: bin
- Local dependencies: tdw-agent, tdw-config, tdw-event, tdw-migration, tdw-protocol, tdw-sql-codegen
- External dependencies: serde_json ^1.0.145
- Dev dependencies: none
- Reverse local dependencies: none
- Feature flags: none
- Test attributes detected: 2
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 8 total, 0 stub-related

## Required Readiness Evidence

- [x] Manifest correctness reviewed.
- [x] Dependency direction reviewed.
- [x] Feature flags reviewed or marked not applicable.
- [x] Public API and error contracts reviewed.
- [x] Runtime behavior reviewed.
- [x] Tests and coverage evidence recorded.
- [x] Docs and examples reviewed.
- [x] Surface wiring reviewed where applicable.
- [x] Scaffold, dead-code, and fallback signals classified.
- [x] Security and reliability risks reviewed.

## Findings

- `xtask` owns offline repository maintenance commands only: schema export/checks, DDL export, migration plan printing, quality gate artifact checks, and the clean-room audit.
- Runtime behavior is intentionally local-file and stdout based; migration commands print plans and do not perform destructive database actions.
- Clean-room audit scans Rust/TOML source for forbidden `finx-`, `tesser-`, and `tdw-provider-openbb` sentinels without introducing forbidden dependencies.
- Scan signals are help/default argument handling, non-destructive scaffold wording, and test `expect` calls; no blocker or copied FinX-XR/OpenBB implementation was found.

## Verification

- Focused xtask evidence passed during workspace verification: `cargo test -p xtask`.
- Final G008 gates passed before cleaner: `cargo fmt --all -- --check`; `cargo check --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo run -p xtask -- clean-room-audit`; `git diff --check`.

## Verdict

Ready with follow-ups. `xtask` is suitable for bootstrap governance and artifact checks; fuller release automation can be added once release packaging is defined.
