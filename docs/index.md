# Docs index

A one-page map of the `docs/` tree. Start here, then drill into the relevant
area.

## Architecture & design

- [`architecture.md`](architecture.md) — overall workspace architecture.
- [`event-spine.md`](event-spine.md) — the shared event spine.
- [`protocol-config-boundary.md`](protocol-config-boundary.md) — protocol/config boundary.
- [`extensibility-backbone.md`](extensibility-backbone.md) — tools / agents / UDF extensibility.
- [`session-rollout-daemon.md`](session-rollout-daemon.md), [`thin-clients-replay.md`](thin-clients-replay.md), [`multi-session.md`](multi-session.md) — session, replay, and daemon model.
- [`parity-layer.md`](parity-layer.md), [`kg-tags.md`](kg-tags.md), [`llm-knowledge.md`](llm-knowledge.md) — feature-layer designs.
- [`adr/`](adr/) — architecture decision records.

## Release & operations

- [`release.md`](release.md) — release process overview.
- [`release/local-stack-runbook.md`](release/local-stack-runbook.md) — bring up the local stack.
- [`release/data-backend-runbook.md`](release/data-backend-runbook.md) — durable backend bootstrap.
- [`release/production-auth-oidc.md`](release/production-auth-oidc.md) — **production OIDC ingress auth (`TDW_OIDC_*`)**: the consolidated contract, fail-closed semantics, and boot diagnostics.
- [`release/worker-deployment.md`](release/worker-deployment.md), [`release/mcp-remote-deployment.md`](release/mcp-remote-deployment.md), [`release/live-stack-smoke.md`](release/live-stack-smoke.md) — service/worker/MCP deployment.
- [`docker.md`](docker.md) — Docker Compose profiles and WSL2 guidance.

## Quality & readiness

- [`quality/crate-readiness/matrix.md`](quality/crate-readiness/matrix.md) — readiness matrix across all crates.
- [`quality/crate-readiness/`](quality/crate-readiness/) — per-crate readiness worksheets (the repo's per-crate documentation convention; there are no per-crate `README.md` files).
- [`quality/production-transport-status.md`](quality/production-transport-status.md), [`quality/production-storage-transports.md`](quality/production-storage-transports.md) — production transport/storage status.
- [`quality/end-to-end-smoke.md`](quality/end-to-end-smoke.md) — end-to-end smoke evidence.
- [`quality-gates.md`](quality-gates.md), [`testing.md`](testing.md) — gates and test conventions.

## Data schemas & SQL

- [`schemas/`](schemas/) — the worksheet-convention domain schema docs (provenance, market data, orders, positions, …).
- [`sql-conventions.md`](sql-conventions.md), [`dbt-guide.md`](dbt-guide.md) — SQL and dbt conventions.

## Contributor workflow

- [`development-workflow.md`](development-workflow.md), [`best-practices.md`](best-practices.md) — day-to-day workflow.
- [`worktrees.md`](worktrees.md), [`github.md`](github.md), [`AGENTS.md`](AGENTS.md) — branching, worktrees, and operational rules.
- [`agent-runtime.md`](agent-runtime.md) — agentic CLI runtime boundary.
