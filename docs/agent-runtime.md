# Agent Runtime

`tdw-agent` is the source of truth for v0.1 agent-facing schemas. It exports
JSON Schema for agent cards, skills, slash-command invocations, content refs,
eval runs, workflow definitions, gotchas, and storage mappings through
`xtask schema-sync`.

Runtime ownership:

- `tdw-agent` owns schema types, parser fixtures, DAG validation, gotcha seeds,
  and schema bundle names.
- `tdw-agent-store` owns persistence-facing storage mappings and in-memory
  contract tests for agent cards, workflows, gotchas, and eval runs.
- `tdw-eval-runner` owns deterministic eval execution and persists metric rows
  through the store contract.
- `tdw-workflow-engine` compiles validated workflow DAGs into executable node
  order.
- `tdw-service-api` and `tdw-mcp` expose the agent tools after schemas, storage,
  evals, and workflow compilation are available.

The Postgres migration `20260521_0004_agent_runtime.sql` maps agent runtime
entities into the `agents` and `evals` schemas. The MCP surface is intentionally
thin: deterministic fixture tools delegate to shared service/runtime crates, and
daemon-backed tools submit `OpEnvelope` work through `tdw-app-client` instead of
copying daemon business logic into MCP.
