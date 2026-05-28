<!-- Parent: ../AGENTS.md -->
<!-- Generated: 2026-05-27 | Updated: 2026-05-27 -->

# docs

## Purpose

Hand-authored architectural documentation, ADRs (Architecture Decision
Records), the 11 BOM schema specs, quality-gate records, and the per-crate
readiness matrix. This is the authoritative location for **why** things are
the way they are; the **what** is in the code and the **when** is in
`../.plans/`.

The schema specs in `docs/schemas/` are paired with Rust structs in
`../crates/tdw-domain/`; a drift between the two is caught by
`cargo run -p xtask -- schema-sync`.

## Key Files

| File | Description |
|------|-------------|
| `architecture.md` | High-level workspace boundary and crate roles. |
| `plans.md` | Pointer to `../.plans/` and historical implementation plans. |
| `worktrees.md` | Worktree creation and teardown convention. |
| `docker.md` | Docker Compose profiles (`minimal`, `full`) and WSL2 notes. |
| `release/` | Release and operator runbooks, including the live data-backend bootstrap. |
| `testing.md` | Unit / integration / property / e2e / adversarial tiers. |
| `quality-gates.md` | Phase-exit gates enforced by `xtask quality-gate`. |
| `github.md` | Remote configuration and branch-protection rules. |
| `dbt-guide.md` | dbt-postgres + dbt-clickhouse dispatch rule and conventions. |
| `sql-conventions.md` | Bronze / silver / gold layering, naming, grants. |
| `event-spine.md` | Layer E (hook + event spine) reference. |
| `agent-runtime.md` | Agent runtime layout and MCP tool surface. |
| `extensibility-backbone.md` | How `tdw-hooks` + `tdw-define` + UDFs compose. |
| `kg-tags.md` | Knowledge-graph and tag-rule reference. |
| `llm-knowledge.md` | LLM + embedding + retrieval layering. |
| `parity-layer.md` | Layer C (snapshots, streams, live, graph, spatial, UDFs) reference. |
| `protocol-config-boundary.md` | `tdw-protocol` vs `tdw-config` split rationale. |
| `perf-history.json` | Bench harness output (regression gate; written by `xtask bench`). |

## Subdirectories

| Directory | Purpose |
|-----------|---------|
| `adr/` | Architecture Decision Records (`0001`…). One file per decision. |
| `schemas/` | The 11 BOM schema specs (`01_market_data.md`…`11_costs_fees.md`) plus `00_provenance.md` recording the curated OSS source list. Generated JSON Schemas land in `schemas/agent/`, `schemas/event/`, `schemas/protocol/`, `schemas/config/`. |
| `quality/` | Quality-gate evidence: phase-exit gates JSON, the crate-readiness matrix, per-crate audit worksheets, and the AI-slop cleanup report. |

## For AI Agents

### Working In This Directory

- **ADRs are append-only.** Decisions superseded by a later ADR are marked
  `Status: Superseded` and the new ADR references the old one — never delete
  or rewrite the old file.
- **Schema specs are the source of truth for `tdw-domain`.** Edit
  `schemas/NN_*.md` first, then update the matching Rust struct, then run
  `cargo run -p xtask -- schema-sync` to refresh the generated JSON Schemas.
- **Generated subdirectories** (`schemas/agent/`, `schemas/event/`,
  `schemas/protocol/`, `schemas/config/`, `perf-history.json`) are written by
  `xtask` commands. Edit the upstream Rust types, not the generated files.
- **Per-crate audit worksheets** (`quality/crate-readiness/<crate>.md`) are
  updated by the owning tranche; reads are encouraged, edits should go
  through the tranche's audit cadence.

### Common Patterns

- Cross-reference ADRs by number: "see ADR-0012" not "see the agentic CLI
  ADR". The number is stable; the title can drift.
- Cross-reference schemas by their two-digit prefix: "schema 04" for
  `news_sentiment`, "schema 11" for `costs_fees`.
- Keep markdown lines under 100 characters where it does not hurt
  readability; tables may exceed.

## Dependencies

### Internal

- `../xtask/` writes into `schemas/agent/`, `schemas/event/`,
  `schemas/protocol/`, `schemas/config/`, `quality/phase-exit-gates.json`, and
  `perf-history.json`.
- `../crates/tdw-domain/` mirrors `schemas/01_*.md` through `schemas/11_*.md`.

<!-- MANUAL: Notes added below this line are preserved on regeneration. -->
