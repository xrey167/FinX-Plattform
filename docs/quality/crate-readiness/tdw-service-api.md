# tdw-service-api Readiness Worksheet

Owner tranche: G007-client-service-mcp-acp-runtime-and-worker-crates - Client, Service, MCP, ACP, Runtime, and Worker Crates.

## Baseline Inventory

- Manifest: crates\tdw-service-api\Cargo.toml
- Target kinds: lib
- Local dependencies: tdw-acp, tdw-actor, tdw-agent, tdw-agent-store, tdw-auth, tdw-auth-oidc, tdw-bus, tdw-cdc, tdw-config, tdw-core, tdw-define, tdw-domain, tdw-embed, tdw-embed-local, tdw-entity-resolver, tdw-eval-runner, tdw-event, tdw-exec, tdw-feature-store, tdw-graph, tdw-hooks, tdw-kg, tdw-knowledge, tdw-llm, tdw-llm-anthropic, tdw-llm-openai-compat, tdw-mask, tdw-outbox, tdw-pipe, tdw-protocol, tdw-provider-fileset, tdw-provider-ws-mock, tdw-provider-yahoo, tdw-replay, tdw-rollout, tdw-runtime, tdw-sandbox, tdw-snapshot, tdw-spatial, tdw-stage, tdw-storage-meilisearch, tdw-storage-qdrant, tdw-storage-s3, tdw-table-format, tdw-tag-rules, tdw-tags, tdw-tools, tdw-tui, tdw-udf, tdw-workflow-engine
- External dependencies: bytes ^1.11.0; serde ^1.0.228 features=[derive]; serde_json ^1.0.145
- Dev dependencies: none
- Reverse local dependencies: tdw-cli, tdw-mcp, tdw-service, tdw-worker
- Feature flags: none
- Test attributes detected: 12
- tests/ directory: no
- README: no
- Examples directory: no
- Scaffold/dead-code/fallback scan signals: 12 total, 0 stub-related

## Required Readiness Evidence

- [ ] Manifest correctness reviewed.
- [ ] Dependency direction reviewed.
- [ ] Feature flags reviewed or marked not applicable.
- [ ] Public API and error contracts reviewed.
- [ ] Runtime behavior reviewed.
- [ ] Tests and coverage evidence recorded.
- [ ] Docs and examples reviewed.
- [ ] Surface wiring reviewed where applicable.
- [ ] Scaffold, dead-code, and fallback signals classified.
- [ ] Security and reliability risks reviewed.

## Findings

- Pending tranche audit.

## Verification

- Pending tranche audit. Record focused crate commands and any workspace commands here.

## Verdict

Pending tranche audit. This baseline worksheet is not a production-readiness attestation yet.
