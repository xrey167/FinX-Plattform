# Crate Readiness Framework

Generated for ultragoal story G001-crate-readiness-rubric-inventory-and-matrix on 2026-05-22 from cargo metadata plus a conservative Rust source scan. Refreshed on 2026-06-07 during G001-crate-by-crate-release-blocker-inven after the workspace grew beyond the original bootstrap roster.

## Scope

- Workspace packages enumerated: 116.
- crates/* package directories enumerated: 115.
- Extra workspace package outside crates/*: xtask.
- Every workspace package is assigned to exactly one downstream ultragoal tranche.
- Per-crate worksheets live next to this file and are intentionally initialized as pending baseline artifacts. G002-G008 must replace pending entries with evidence while hardening each crate.

## Rubric

Each per-crate worksheet must end with an explicit production-readiness verdict and evidence for these checks:

1. Manifest correctness: package metadata, publish setting, edition, rust-version inheritance, license, target kinds, and workspace dependency usage are intentional.
2. Dependency direction: local dependencies point inward through accepted architecture layers; shared crates do not depend on higher-level service or client crates.
3. Feature flags: optional behavior is behind explicit features when needed, defaults are deliberate, and feature names are documented.
4. Public API and errors: exported types/functions are minimal and stable for the crate role; errors are typed and do not hide corrupted state or unsupported behavior.
5. Runtime behavior: async/blocking boundaries, persistence, network, filesystem, process, and configuration behavior are deterministic or explicitly guarded.
6. Tests and coverage evidence: unit, integration, golden, property, or mocked tests cover the contract that the crate claims to provide.
7. Docs and examples: crate docs, README material, examples, schema files, or higher-level docs explain intended use and limitations.
8. Surface wiring: service-api, CLI, MCP, TUI, worker, xtask, migrations, or dbt surfaces consume the crate where the architecture says they should.
9. Scaffold, dead-code, and fallback scan: TODOs, bootstrap stubs, mock-only paths, masking defaults, and panic/unwrap/expect use are classified as test-only, fixed, or recorded blockers.
10. Security and reliability: credential handling, injection boundaries, tenant/session isolation, persistence integrity, and failure modes are reviewed for the crate role.
11. Verdict: Ready, Ready with follow-ups, Blocked, or Not production-ready, with durable blocker links when not ready.

## Required Workflow For G002-G008

1. Open the worksheet for each crate assigned to the active tranche.
2. Replace baseline fields with direct evidence from manifests, source, tests, docs, and verification output.
3. Add or fix implementation/tests/docs only where the evidence shows a real production-readiness gap.
4. Update matrix.md and dependency-topology.md if a manifest, dependency, feature, or verdict changes.
5. Run `cargo run -p xtask -- crate-readiness-check` before release or whenever crates are added or removed.
6. Keep clean-room constraints intact: no finx-* crates or dependencies, no copied FinX-XR code, and no tdw-provider-openbb.

## Files

- matrix.md: aggregate coverage and verdict matrix.
- dependency-topology.md: baseline local dependency and feature-flag topology.
- <crate>.md: one worksheet per workspace package.
