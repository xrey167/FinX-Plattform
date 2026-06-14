# tdw-partner Readiness Worksheet

Generated during the partner-system W2 build (Partner Core — the shared conversational front door).

## Evidence Snapshot

- Manifest: `crates/tdw-partner/Cargo.toml`.
- Targets: lib.
- Features: none.
- Local deps: tdw-core, tdw-domain, tdw-endpoint-catalog, tdw-knowledge, tdw-llm, tdw-openbb-agent, tdw-tags, tdw-taxonomy.
- Reverse deps: tdw-cli, tdw-mcp, tdw-service-api (the three thin surface adapters).
- Test attributes found in Rust sources: 13.
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

## Verdict

Ready with follow-ups. New crate landed with the W2.1–W2.8 surface (core types,
DataPlane port, catalog-bounded resolver, turn sequencer, gated write-back, and the
MCP/Workspace/CLI adapters) and offline-deterministic tests covering each.
