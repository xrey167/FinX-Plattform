# ADR-0009: License And Publication Boundary

## Decision

FinX-Plattform is a private personal codebase. Workspace crates are marked
`publish = false` and `license = "UNLICENSED"` so Cargo tooling and dependency
audits treat local path crates as private artifacts, while third-party
dependencies remain subject to `cargo deny check`.

## Consequences

- No workspace crate is intended for crates.io publication in v0.1.
- `cargo deny check` enforces advisories, allowed third-party licenses, and
  source policy for external dependencies.
- Private path dependency wildcard warnings are tolerated until the workspace
  dependency table is normalized.
