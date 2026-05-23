# ADR-0012: Agentic CLI Runtime Boundary

## Status

Accepted for the agentic CLI ultragoal.

## Context

The brainstorm in `.omx/ultragoal/brief.md` maps patterns from Codex,
opencode, goose, aider, and other public agentic CLI projects onto the current
FinX-Plattform Rust workspace. The target repo is already a clean-room TDW
workspace with many `tdw-*` crates, so the first step is inventory and boundary
selection, not bulk crate creation.

The active clean-room contract still applies:

- no `finx-*` crates or dependencies;
- no copied FinX-XR code, trait signatures, tests, or module contents;
- no `tdw-provider-openbb`;
- public agentic projects may be studied for architecture patterns only.

## Current Workspace Inventory

Existing crates that should be reused as the substrate:

| Area | Existing crates | Decision |
| --- | --- | --- |
| Domain and provider contracts | `tdw-core`, `tdw-domain`, provider crates | Keep the current data-provider and storage traits while `tdw-protocol` is introduced. Do not create a provider-core split in the first pass. |
| Runtime shells | `tdw-runtime`, `tdw-cli`, `tdw-service`, `tdw-service-api`, `tdw-worker` | Reuse as the first headless/service shells. New client/server crates are added only where the daemon boundary needs a sharper dependency cut. |
| Event spine | `tdw-event`, `tdw-actor`, `tdw-bus`, `tdw-hooks`, `tdw-outbox`, `tdw-cdc`, `tdw-replay` | Extend these rather than replacing them. `tdw-protocol::EventMsg` will wrap or reference the stable event envelope instead of duplicating it. |
| Agent schemas | `tdw-agent`, `tdw-agent-store`, `tdw-workflow-engine`, `tdw-eval-runner` | Keep these as schema/workflow/eval owners. They become tools and MCP resources, not the core event protocol. |
| UDF and sandbox candidates | `tdw-udf`, `tdw-udf-external`, `tdw-udf-js`, `tdw-udf-python`, `tdw-udf-wasm` | Add one adapter crate, `tdw-sandbox`, and plug these crates into it. Do not add a JS or Lua plugin runtime. |
| Retrieval and knowledge | `tdw-embed-*`, `tdw-storage-qdrant`, `tdw-storage-meilisearch`, `tdw-kg`, `tdw-tags`, `tdw-tag-rules`, `tdw-feature-store`, `tdw-entity-resolver` | Reuse for retrieval. Add `tdw-knowledge` only as an orchestration facade over existing KG, tag, embedding, and storage crates. |
| MCP surface | `tdw-mcp` | Keep as the initial local MCP binary. Split into `tdw-mcp-server` and `tdw-mcp-client` only when inbound and outbound MCP contracts are both implemented. |
| Persistence and rollout | storage crates, dbt, migrations | Add focused session and rollout crates rather than overloading storage adapters with agent session state. |

## New Crate Decisions

Create these crates during the ultragoal when their story reaches them:

- `tdw-protocol`: pure serializable `Op`, `EventMsg`, IDs, errors, approval,
  tool-call, queue, and replay contracts. It has no dependency on agent core,
  runtime, storage adapters, or service shells.
- `tdw-config`: layered config ownership with schema emission. It owns the
  precedence contract: user defaults, env-pointed file, project config,
  split-file config, inline env JSON, and CLI flags.
- `tdw-tools`: `Tool`, `ToolRegistry`, `ToolRouter`, and later
  `ToolOrchestrator`.
- `tdw-sandbox`: one `SandboxRuntime` and `UdfRequest` adapter over the
  existing UDF host crates.
- `tdw-session`: SQLx/SQLite hot session state, permission rules, pending
  approvals, and session metadata.
- `tdw-rollout`: append-only JSONL archive and replay helpers for agent runs.
- `tdw-llm`, `tdw-llm-anthropic`, `tdw-llm-openai-compat`: a small in-house
  model trait plus concrete adapters. Do not adopt `rig`, `swiftide`,
  `llm-chain`, or `kalosm` wholesale.
- `tdw-acp`: outward Agent Client Protocol boundary for future IDE/TUI clients.
- `tdw-app-server` and `tdw-app-client`: daemon and thin-client split over UDS
  first, with HTTP+SSE added after the local daemon contract is stable.
- `tdw-knowledge`: orchestration over existing KG, tags, embeddings, and
  warehouse metadata. It must not replace those lower-level crates.

Defer these until the substrate is stable:

- `tdw-tui`: final ratatui client, consuming `EventMsg` streams only.
- `tdw-exec`: either a new headless crate or a `tdw-cli exec` split after
  `tdw-app-client` exists.
- `tdw-mcp-client` and `tdw-mcp-server`: split from `tdw-mcp` only when the
  inbound and outbound MCP surfaces need independent dependencies.

Do not create:

- `tdw-provider-openbb`;
- `finx-*` crates;
- a bespoke JS or Lua plugin-runtime crate;
- an actor-framework wrapper crate for the core loop.

## Resolved Open Questions

### Query Planning Boundary

Query planning sits outside the model loop and inside TDW runtime/service
contracts. The agent may propose a query-oriented `Op`, but deterministic
planning, validation, rewrite, and execution are owned by TDW crates. The model
does not directly issue warehouse SQL against storage engines.

Initial boundary:

- `tdw-protocol` defines user-facing `Op` values such as query request, ingest
  request, tool call, approval response, and cancel.
- `tdw-runtime` translates validated operations into provider/storage actions.
- `tdw-rewrite`, `tdw-hooks`, and storage crates participate through typed
  runtime calls and events.
- `tdw-core` keeps provider/storage traits until protocol migration is complete.

### Cost Accounting Unit

Cost accounting is recorded per operation and aggregated per session. The ledger
unit is a `CostLedgerEntry` keyed by session ID and operation ID. It tracks at
least tokens, model provider, tool/runtime wall time, bytes scanned, rows read,
rows written, and storage/backend name. This prevents token-only accounting from
being mistaken for warehouse cost.

`tdw-session` owns hot cost state. `tdw-rollout` archives the append-only JSONL
record. Later warehouse tables can mirror those records for analysis.

### MCP Server Identity

The MCP surface exposes role-scoped TDW tools, not one opaque mega-agent tool as
the only interface. The initial surface should keep stable namespaces such as:

- `tdw.query.plan`
- `tdw.query.run`
- `tdw.ingest.run`
- `tdw.agent.run`
- `tdw.kg.search`
- `tdw.udf.run`

Each tool advertises its capability, required permissions, and audit category.
An agent-style convenience tool can be added later, but it must route through
the same tool registry and permission envelope.

### Hook HTTP Authentication

HTTP hooks are not enabled by default. The first hook expansion should support
local command and prompt/context handlers before remote HTTP is considered.

When HTTP handlers are enabled, they require all of the following:

- explicit allowlist in project config;
- HTTPS URL unless a local development override is active;
- signed request body with a project/session secret or equivalent credential;
- bounded timeout, response-size cap, and retry policy;
- no implicit credential forwarding;
- explicit permission to veto through `should_stop`.

Unauthenticated HTTP hook results are advisory only and cannot stop execution or
inject privileged context.

## Implementation Order

The ultragoal story order is the implementation contract:

1. Inventory, decisions, and clean-room boundaries.
2. Protocol, config, and core boundary.
3. Hook event spine, permissions, and prompts.
4. Tools, UDF sandbox, MCP, and ACP.
5. Session, rollout, daemon, and queue loop.
6. LLM, retrieval, and knowledge intelligence.
7. Thin clients, CLI, TUI, service, and replay.
8. Integration, verification, and final quality gate.

## Consequences

- Existing crate stubs remain useful; the next stories should adapt them rather
  than replacing them with parallel abstractions.
- The new protocol/config/session/rollout crates are justified because the
  current workspace does not yet have an I/O-free operation protocol or durable
  agent session ledger.
- Clients must move toward the protocol boundary and away from directly
  depending on agent/runtime internals.
- The clean-room boundary is preserved even when external agentic CLI projects
  are used as architectural references.
