# Architecture

FinX-Plattform is the private Rust implementation repo for the TDW plan set.

The project is organized as one Cargo workspace with explicit `tdw-*` crates. The
first stable boundary is:

- `tdw-core`: shared traits, envelope, errors, provider registry, and storage
  contracts.
- `tdw-domain`: canonical Rust structs derived from the 11 BOM schema specs.
- `tdw-runtime`: command orchestration shared by service, worker, CLI, and MCP.
- `tdw-test-utils`: deterministic fixtures and future container helpers.
- `xtask`: repository maintenance and verification commands.

Later crates are present as compile-ready stubs so work can begin in parallel without
renaming or path churn.

## Clean-Room Boundary

FinX-XR can be read only for high-level pattern awareness when a plan asks for it.
Do not copy code, trait signatures, tests, or module contents from it. `tdw-provider-openbb`
is intentionally absent because OpenBB is inspiration only, not a bridge dependency.
