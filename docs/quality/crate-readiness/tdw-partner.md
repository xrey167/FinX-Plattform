# tdw-partner Readiness Worksheet

Generated during the partner-system W2 build (Partner Core — the shared
conversational front door) and extended in W3 (the proactive layer: brief +
nudges).

## Evidence Snapshot

- Manifest: `crates/tdw-partner/Cargo.toml`.
- Targets: lib.
- Features: none.
- Local deps: tdw-core, tdw-domain, tdw-endpoint-catalog, tdw-knowledge, tdw-llm, tdw-openbb-agent, tdw-tags, tdw-taxonomy.
- Dev-only deps (cycle-break): tdw-cron, tdw-protocol, chrono — used solely by the
  W3.3 pinned-clock scheduled-fire test, which exercises the real `tdw-cron` spine.
  A normal dep on `tdw-cron` would form a cycle (tdw-cron → tdw-worker →
  tdw-service-api → tdw-partner), so the production schedule trigger is assembled
  by the daemon facade (`tdw-backend`) from the pure `BriefJobSpec` this crate exports.
- Reverse deps: tdw-cli, tdw-mcp, tdw-service-api (the three thin surface adapters).
- Test attributes found in Rust sources: 29 (W2 + the W3 proactive/scheduler suites).
- Tests directory: False (unit + offline-stub tests inline per module).
- Docs/examples: crate-readiness worksheet only.
- Scan signal files: 0.

## Release Assessment

- A leaf orchestration crate (`finx-partner` §1.1): it composes `KnowledgeRuntime`'s
  gated proposal queue, the endpoint catalog, the `tdw-llm` streaming model, and a
  `DataPlane` port over the dispatcher behind one surface-agnostic `PartnerCore::turn`
  sequencer. It depends on the pure leaves it needs and is depended on only by the
  three thin adapters, keeping the dependency DAG clean (no `tdw-knowledge` ↔
  `tdw-service-api` bidirectional edge).
- Anti-over-engineering line honored (`finx-partner` §9 DROPs): no new planner / agent
  loop / orchestration DSL — `turn` is a sequencer; route resolution is catalog-bounded
  LLM tool-selection guarded by `is_valid_route`, never free-form dispatch.
- Autonomy is audit-only (directive 2): the write-back step submits through the gate
  (`ProposalQueue::submit` refuses below `Adaptivity::Learning`) and does not block on a
  human; a unit test pins the refusal.
- No clean-room exception is recorded for this crate.
- Any code-level follow-up remains non-blocking unless `fmt`, `clippy -D warnings`,
  tests, clean-room audit, or `crate-readiness-check` fails.

## W3 — Proactive layer (brief + nudges)

- `proactive.rs`: the `Nudge` model + the **pure** `build_brief` assembler over
  `BriefInputs` (unified fired-alert + knowledge signals), ranked by
  severity × recency (determinism is the W3.1/W3.2 gate), plus
  `rerank_with_dismissals` (the W3.5 dismissal-driven re-rank; the gated
  `tdw.kg.feedback` → self_tune/lessons routing is done by the surface adapter).
- `scheduler.rs`: the pure `BriefJobSpec` + `daily_brief_spec`; the W3.3
  pinned-clock scheduled-fire test drives the real `tdw-cron` `due_triggers` /
  `build_job` spine (no new scheduler).
- Surfaces (W3.6): `tdw.partner.brief` MCP tool (in `tdw-mcp`, gated on the same
  PartnerCore) + `tdw partner brief` CLI. The Workspace brief widget is deferred
  to W6 per the design (the widget contract is enumerated from the catalog).

## Verdict

Ready with follow-ups. New crate landed with the W2.1–W2.8 surface (core types,
DataPlane port, catalog-bounded resolver, turn sequencer, gated write-back, and the
MCP/Workspace/CLI adapters) and the W3 proactive layer (brief assembler + nudge
model + cron schedule seam + MCP/CLI surfaces), all with offline-deterministic
tests. The W2 Gemini #438 review fixes (parameterized route fetch, non-const
`Provenance::is_empty`, robust route-selection trimmer) are folded in.
