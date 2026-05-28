<!-- Parent: ../AGENTS.md -->
<!-- Generated: 2026-05-27 | Updated: 2026-05-27 -->

# adr

## Purpose

Architecture Decision Records. Each file is a frozen rationale for one
decision: what was decided, why, what was considered and rejected, what the
consequences are. ADRs are **append-only** — a decision superseded by a later
one is marked `Status: Superseded` and the new ADR back-references it.

## Numbering

ADRs are zero-padded four-digit numbers (`0001`, `0002`, …) and a
kebab-case slug. The next number is "max existing + 1", with no gaps. When in
doubt, list this directory and add one above the maximum.

## Key Files

| File | Decision |
|------|----------|
| `0001-relationship-to-finx-xr.md` | Clean-room boundary versus FinX-XR; `finx-*` crates and `tdw-provider-openbb` are forbidden. |
| `0002-workspace-bootstrap.md` | Historical bootstrap decision for early placeholder crates. New cleanup work should prefer concrete contracts over placeholders. |
| `0003-github-and-worktree-policy.md` | Branch naming (`work/<topic>`, etc.), worktree lifecycle, PR + squash-merge rules. |
| `0009-license-personal-private.md` | License = personal, private codebase. Not OSS, not commercial. |
| `0011-dbt-dispatch-rule.md` | dbt-postgres for OLTP-shaped data, dbt-clickhouse for time-series; per-model `target=` dispatch macro. |
| `0012-agentic-cli-runtime-boundary.md` | Agentic CLI runtime crate boundary — `tdw-acp`, `tdw-app-server`, `tdw-app-client`, `tdw-exec`, `tdw-tui`. |

## For AI Agents

### Working In This Directory

- **Never rewrite an existing ADR.** Mark it superseded and write a new one.
- One decision per file. If you find yourself describing two decisions in one
  ADR, split it.
- Keep ADRs short — the standard sections are: Status, Context, Decision,
  Consequences, optional Alternatives Considered. A reader should grasp the
  decision in two minutes.
- Link to the originating plan in `../../.plans/` if the decision was driven
  by a specific design pass.

<!-- MANUAL: Notes added below this line are preserved on regeneration. -->
