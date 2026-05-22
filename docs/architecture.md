# Architecture

FinX-Plattform is the private Rust implementation repo for the TDW plan set.

The project is organized as one Cargo workspace with explicit `tdw-*` crates. The
first stable boundary is:

- `tdw-core`: shared traits, envelope, errors, provider registry, and storage
  contracts.
- `tdw-protocol`: I/O-free operation, event, approval, queue, and replay
  contracts used by services and future clients.
- `tdw-config`: layered TDW configuration and schema output.
- `tdw-domain`: canonical Rust structs derived from the 11 BOM schema specs.
- `tdw-runtime`: command orchestration shared by service, worker, CLI, and MCP.
- `tdw-tools`: tool registry, router, and orchestrator contracts.
- `tdw-sandbox`: adapter over the existing TDW UDF runtimes.
- `tdw-acp`: outward Agent Client Protocol boundary for future IDE/TUI clients.
- `tdw-session`: SQLx/SQLite-backed hot session, permission, approval, and cost
  state.
- `tdw-rollout`: append-only JSONL replay archive.
- `tdw-app-server` and `tdw-app-client`: daemon endpoint, submission queue, and
  thin-client contracts.
- `tdw-llm`, `tdw-llm-anthropic`, and `tdw-llm-openai-compat`: small language
  model trait and deterministic adapter contracts.
- `tdw-knowledge`: retrieval facade over embeddings, vector storage, KG, tags,
  and syntax summaries.
- `tdw-exec`: headless protocol-event execution path.
- `tdw-tui`: ratatui event-line renderer over `EventMsg` streams.
- `tdw-test-utils`: deterministic fixtures and future container helpers.
- `xtask`: repository maintenance and verification commands.

Later crates are present as compile-ready stubs so work can begin in parallel without
renaming or path churn.

The agentic CLI runtime boundary and crate-selection decisions are recorded in
`docs/adr/0012-agentic-cli-runtime-boundary.md`.

## Clean-Room Boundary

FinX-XR can be read only for high-level pattern awareness when a plan asks for it.
Do not copy code, trait signatures, tests, or module contents from it. `tdw-provider-openbb`
is intentionally absent because OpenBB is inspiration only, not a bridge dependency.
