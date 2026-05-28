<!-- Parent: ../AGENTS.md -->
<!-- Generated: 2026-05-27 | Updated: 2026-05-27 -->

# quality

## Purpose

Quality-gate evidence and the per-crate readiness matrix. Phase exits are
governed by `phase-exit-gates.json`; per-crate readiness is governed by the
matrix and per-crate worksheets. AI-slop cleanup audit output lives here too.

## Key Files

| File | Description |
|------|-------------|
| `phase-exit-gates.json` | The 17-gate phase-exit policy — written by `cargo run -p xtask -- quality-gate write`, verified by `xtask quality-gate check`. Do not hand-edit. |
| `ai-slop-cleanup-report.md` | Findings from the AI-slop / deslop sweep. |

## Subdirectories

| Directory | Purpose |
|-----------|---------|
| `crate-readiness/` | Per-crate audit worksheets and the cross-crate matrix. |

## For AI Agents

### Working In This Directory

- **`phase-exit-gates.json` is generated.** Edit `quality_gates()` in
  `../../xtask/src/main.rs` and rerun
  `cargo run -p xtask -- quality-gate write` to refresh it.
- The 17 gates partition into tiers: `lint`, `test`, `coverage`, `schema`,
  `performance`, `security`, `governance`, `release`, `mutation`,
  `stability`. Phase-exit gates have `requiredForPhaseExit: true`; nightly
  gates do not.
- AI-slop reports are appended-to, not rewritten. Add new findings as new
  sections with dates.

<!-- MANUAL: Notes added below this line are preserved on regeneration. -->
