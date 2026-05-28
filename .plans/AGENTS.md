<!-- Parent: ../AGENTS.md -->
<!-- Generated: 2026-05-27 | Updated: 2026-05-27 -->

# .plans

## Purpose

Frozen design-time planning artifacts. Each file is a multi-section plan
(RALPLAN-DR shape: principles, drivers, options, acceptance criteria, phases,
risks, verification, ADR header, open questions) backing a phase or layer of
the project. `.plans/` is **append-only** — a plan superseded by a later
revision adds a new file and back-references the old one in its changelog.

The summary index lives in `../docs/plans.md`.

## Key Files

| File | Layer / Phase |
|------|---------------|
| `2026-05-21-rust-trading-data-warehouse.md` | Core plan (Phases 0–6) — workspace, Fetcher/Streamer, storage engines, providers, shells. v2.1 with all open questions resolved. |
| `2026-05-21-data-engineering-and-agent-schemas.md` | Layer A + B (Phases 7–8) — dbt medallion, DDL codegen, agent schemas, eval runner. |
| `2026-05-21-hook-event-spine.md` | Layer E (Phase 9) — two-lane hook & event spine; renumbers later phases. |
| `2026-05-21-databend-surrealdb-feature-parity.md` | Layer C (Phases 10–14, post-renumber) — snapshots, streams, graph, spatial, stages, UDFs, DEFINE, auth. |
| `2026-05-21-connect-rust-buffa-evaluation.md` | Evaluation pass on `buffrs` / `connect-rust`; no phase changes. |
| `2026-05-21-test-strategy.md` | Test tiering and verification cadence. |
| `2026-05-21-knowledge-graph-and-tags.md` | KG + tags design pass. |

## For AI Agents

### Working In This Directory

- **Plans are frozen at commit time.** If a plan needs revision, write a new
  dated file and note in its `## Changelog` that it supersedes the prior file.
  Do not edit the old plan in place.
- Filenames are `YYYY-MM-DD-<kebab-case-slug>.md`. Multiple plans on the same
  date are fine.
- Plans cross-reference each other by relative path
  (`./2026-05-21-rust-trading-data-warehouse.md`). Keep filenames stable
  because of that.
- When implementing from a plan, link the implementing commit back to the
  plan section (e.g. "implements §4 storage trait split").

## Dependencies

### Internal

- `../docs/plans.md` — index pointing at this directory.
- `../docs/adr/` — ADRs are the post-commit, frozen distillation of a plan's
  Decision section.

<!-- MANUAL: Notes added below this line are preserved on regeneration. -->
