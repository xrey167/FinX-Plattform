# Agentic CLI Patterns Worth Lifting into FinX-Plattform (Rust)

**Date:** 2026-05-22
**Scope:** Deep-dive synthesis across OpenAI Codex (Rust), sst/opencode (TS+Electron),
charmbracelet/crush (Go), block/goose (Rust), aider (Py), continuedev/continue (TS),
google-gemini/gemini-cli (TS), charmbracelet/mods (Go), and Claude Code's public surface.
**Target:** the existing FinX-Plattform Rust workspace (TDW = trading data warehouse) with
planned hook-event-spine, agent-schemas, MCP, knowledge-graph-and-tags, UDF runtimes.

---

## TL;DR — what to actually do

**Adopt now (high leverage, low risk):**

1. **`tdw-protocol` crate** — pure data-model, no I/O, owning `Op` + `EventMsg` enums. Every
   other crate depends on it; nothing depends on agent core. (Codex's `codex-rs/protocol`.)
2. **Submission-Queue / Event-Queue loop** — interrupts, approvals, tool calls all flow as
   serializable `Op`s; outputs flow as `EventMsg`s. No side channels. Makes everything
   recordable, replayable, and protocol-versionable. (Codex.)
3. **Daemon + thin clients over UDS/HTTP+SSE** — TUI, CLI (`tdw exec`), MCP server, future
   web UI all speak the same `protocol`. (Codex's app-server + opencode's HTTP+SSE.)
4. **Registry → Router → Orchestrator tool stack** — three layers, with orchestrator owning
   the `approve → sandbox → run → retry` envelope per call. UDFs become tools. (Codex.)
5. **Two-protocol backbone: ACP outward + MCP inward** — speak Agent-Client-Protocol to
   frontends (Zed-originated, used by goose), speak Model-Context-Protocol to extensions and
   sub-agents. Don't invent a third protocol. (goose.)
6. **Hook event spine = Claude-Code-shape over Codex's `HookRuntimeOutcome`** — typed enum of
   lifecycle events; handlers as one of `Command | Http | Mcp | Prompt | Agent`. Hooks both
   veto (`should_stop`) and inject (`additional_contexts`). This directly satisfies the
   existing `hook-event-spine` plan.
7. **SQLite via `sqlx` for session/permission state + JSONL for rollouts** — opencode already
   migrated away from JSON-per-session; start where they landed. JSONL rollouts give cheap
   replay + crash-safety. (opencode + Codex hybrid.)
8. **Tool prompt text as sibling `.txt` files** — model-facing prose lives alongside each
   tool's source. Prompt iteration becomes a diff. (opencode.)
9. **Last-match-wins permission rules + `oneshot` deferred approvals** — rules are
   `{ permission, pattern, action }` evaluated last-match-wins; `ask` parks a `oneshot::Sender`
   keyed by `PermissionID`. Any client can resolve it. (opencode.)
10. **Embedded LanceDB or Qdrant for embeddings + `tree-sitter` for syntactic structure** —
    hybrid retrieval (no LSP dependency) for the planned knowledge-graph-and-tags work.

**Reject / skip (popular but wrong for this codebase):**

- A bespoke JS or Lua plugin runtime. Make user-supplied tools **MCP stdio servers**. One
  extensibility path, not two. (opencode's TS plugins are a cautionary tale.)
- Adopting `rig` / `swiftide` / `llm-chain` / `kalosm` wholesale. Define a small in-house
  `LanguageModel` trait. Cover Anthropic Messages + OpenAI-compatible; absorb the long tail
  via `base_url` swap. (See research §LLM Client SDKs.)
- An actor framework (`ractor`, `kameo`) for the agent loop. Hand-rolled `tokio::select!`
  beats actors at <10 long-lived tasks. (Codex, goose, aider all hand-roll.)
- `sled` for any persistence. Use `sqlx`+SQLite or `redb`. The `sled` maintainer himself
  recommends SQLite for reliability.
- `reqwest-eventsource` (last release March 2024). Use `eventsource-client` instead.
- `deno_core` for embedding. 358 breaking releases; docs warn it isn't for external use.
  Prefer `rquickjs` for JS UDFs, `wasmtime` as the default UDF host.

---

## 1. Architectural backbone — the SQ/EQ + daemon model

**Pattern source:** Codex `codex-rs/protocol` + `codex-rs/app-server*`.

The single highest-leverage idea in this entire survey is **one I/O-free `protocol` crate that
defines an `Op`/`EventMsg` pair and a Submission/Event queue**. Codex makes this work across:

- TUI (`codex-rs/tui` does NOT depend on `core`)
- Headless exec (`codex-rs/exec`)
- MCP server exposing the agent to other agents (`codex-rs/mcp-server`)
- Future frontends

The seam is enforced by the dependency graph: `protocol` has no internal deps, so any crate
that depends on it cannot bring in agent internals.

**FinX mapping.** Create `crates/tdw-protocol/` early. Owns:

```rust
pub enum Op {
    RunQuery { sql: String, plan_id: PlanId },
    IngestBatch { provider: ProviderName, range: TimeRange },
    RegisterUdf { name: String, source: UdfSource },
    Approval { id: ApprovalId, decision: ApprovalDecision },
    Interrupt,
    Compact,
    Shutdown,
}

pub enum EventMsg {
    QueryPlan { plan: Plan, cost_estimate: Cost },
    QueryRowDelta { rows: RecordBatch, plan_id: PlanId },
    ProviderProgress { ingested: u64, total: Option<u64> },
    UdfCompiled { name: String, sandbox: SandboxKind },
    ApprovalRequest { id: ApprovalId, kind: ApprovalKind, payload: Value },
    HookInjected { stage: HookStage, context: String },
    Warning { code: WarnCode, message: String },
    TurnComplete { tokens: TokenUsage, cost: Cost },
}
```

Then `crates/tdw-app-server/` owns the daemon, `crates/tdw-app-client/` is the thin client
crate (used by TUI, CLI, MCP server). UDS on Unix, named pipe on Windows. Same wire format.

**Concrete next step.** The existing plans `data-engineering-and-agent-schemas.md` and
`hook-event-spine.md` should be unified under this `tdw-protocol` crate before either gets
implemented further. They're describing different facets of the same envelope.

---

## 2. Tool / UDF stack — three layers

**Pattern source:** Codex `core/src/tools/{registry,router,orchestrator}.rs`.

```
ToolRegistry        HashMap<ToolName, Arc<dyn CoreToolRuntime>>     (what exists)
   ↓
ToolRouter          dispatch + parallel/serial decisions             (how to fan out)
   ↓
ToolOrchestrator    approval → sandbox → run → retry-no-sandbox      (per-call envelope)
   ↓
Tool::execute       the actual work
```

For FinX this maps 1:1 onto **UDFs as tools**:

- `crates/tdw-udf-external` / `-js` / `-python` / `-wasm` already exist — each becomes one
  `CoreToolRuntime` implementor.
- The orchestrator's "retry without sandbox if policy allows" pattern becomes "retry without
  acceleration" or "fall back from WASM to subprocess" — same shape, different policy.
- The `OrchestratorRunResult<Out>` carries `deferred_*` fields (deferred side effects). For a
  data warehouse those are deferred writes / journal entries.

**JSON Schema.** Codex hand-builds `ToolSpec`; opencode uses Effect schema. Both are wrong for
Rust. Use **`schemars` derive** for the UDF parameter types, then build the model-facing
envelope (OpenAI / Anthropic tool block) once in the registry layer. This is one of the
gotchas the ecosystem map called out — every Rust agent framework re-rolls this layer.

---

## 3. MCP + ACP — the two-protocol backbone

**Pattern source:** block/goose `Cargo.toml` (`rmcp = "1.7"`, `agent-client-protocol = "0.11"`).

- **Inward (extensions):** speak **MCP** to plug in third-party tools. `rmcp` (the official
  Rust SDK, 2.19M DL/mo, version 1.7 as of May 2026) covers stdio + Streamable HTTP. Codex
  also uses `rmcp` and even exposes itself as an MCP server (`codex-rs/mcp-server`).
- **Outward (frontends):** speak **ACP** (Agent Client Protocol, originated at Zed) so the
  agent core can be embedded in TUI, IDE plugins, web. goose already does this.

This is the **single best architectural decision in this entire survey**. It collapses what
would otherwise be three or four bespoke protocols into two open standards.

**FinX mapping.** Two new crates:

- `crates/tdw-mcp-client/` — wraps `rmcp` for outbound calls to third-party MCP servers
  (think: "user wants the agent to call a proprietary risk-model tool"). Wire `RmcpClient`
  the way Codex does (see `codex-rs/codex-mcp/`).
- `crates/tdw-mcp-server/` — expose TDW itself as MCP tools (`tdw.query`, `tdw.ingest`,
  `tdw.list_providers`). Lets any agentic client drive TDW directly. Steal the
  `MessageProcessor` pattern from Codex's `mcp-server/message_processor.rs` verbatim — it's
  the right shape.
- `crates/tdw-acp/` — implement ACP server so a TUI or IDE plugin can drive an agent that
  itself drives TDW.

---

## 4. Hook event spine — concrete proposal

**Pattern sources:** Claude Code's hook matrix (richest), Codex `codex-rs/hooks/` (cleanest
Rust implementation), gemini-cli `hooks/` package.

The existing `hook-event-spine.md` plan should land as `crates/tdw-hooks/` with:

**Events** (typed enum, not strings):
```rust
pub enum HookEvent {
    SessionStart, SessionEnd,
    UserPromptSubmit { text: String },
    PreToolUse { tool: ToolName, args: Value },
    PostToolUse { tool: ToolName, result: ToolResult },
    PostToolUseFailure { tool: ToolName, error: ToolError },
    PreCompact, PostCompact,
    PermissionRequest { kind: PermissionKind, payload: Value },
    QueryPlanReady { plan: Plan, cost: Cost },     // FinX-specific
    UdfRegistered { name: String, sandbox: SandboxKind },  // FinX-specific
    IngestStart { provider: ProviderName },        // FinX-specific
    IngestComplete { provider: ProviderName, rows: u64, cost: Cost },
}
```

**Handler kinds** (taken from Claude Code's five-kind model):
```rust
pub enum Handler {
    Command { cmd: String, args: Vec<String> },     // exec, JSON via stdin/stdout
    Http { url: Url, headers: HeaderMap },          // POST JSON, get JSON back
    Mcp { server: McpServerName, tool: ToolName },  // call a known MCP tool
    Prompt { template: String, model: ModelId },    // LLM-evaluated guard
    Agent { agent: AgentName, prompt: String },     // sub-agent verification
}
```

**Outcome** (Codex's `HookRuntimeOutcome`):
```rust
pub struct HookOutcome {
    pub should_stop: bool,
    pub additional_contexts: Vec<String>,
    pub modifications: Vec<HookModification>,  // FinX adds: rewrite the SQL plan
}
```

The `Prompt` and `Agent` handler kinds are the differentiator vs simple command-only hook
systems — they let users write *LLM-evaluated* guards ("is this query likely to be cost-bombing
the warehouse?"). Claude Code is the only tool surveyed that exposes these as first-class.

---

## 5. Sandbox & UDF — per-platform crates with one adapter

**Pattern source:** Codex's `linux-sandbox` / `windows-sandbox-rs` / `bwrap` / `execpolicy`
all as separate crates, unified by an `ExecRequest` adapter in
`core/src/sandboxing/mod.rs`.

**FinX is in great shape here already** — the workspace already has `tdw-udf-wasm` and
`tdw-udf-external` and `tdw-udf-python` and `tdw-udf-js` as separate crates, which is the
right structure. Add the adapter:

- `crates/tdw-sandbox/` — owns `UdfRequest` and the four impl crates implement
  `trait SandboxRuntime`. No `#[cfg(target_os)]` in user-facing code.
- Default to **`wasmtime` (45.0)** as the primary UDF host. Component Model + WASI 0.2 +
  fuel/epoch timeouts give the right semantics for a data warehouse.
- `pyo3` (0.28) only when users *demand* CPython for numpy/pandas — be honest that one
  Python world per process is the architectural ceiling. Long-term, run Python UDFs in a
  **sidecar process pool** (the Go/TS workaround), not in-process.
- `rquickjs` (0.11) for JS UDFs — Promise → Future bridge is async-native; far saner than
  `deno_core`.
- Linux process sandbox via `landlock` (0.4.4) wrapping the agent itself, not UDFs (UDFs
  go through wasmtime).
- macOS / Windows: shell out to `sandbox-exec` / use raw AppContainer APIs. There is **no
  good crate** here — accept the platform debt; it's a wash with Go/Node.

---

## 6. Session, history, rollouts

**Hybrid recommendation** combining opencode's SQLite migration with Codex's JSONL rollouts:

- **Hot state** (active session: messages, parts, permission rules, share metadata) →
  `sqlx` + SQLite. opencode's `session/session.sql.ts` schema is a strong starting template:
  parent/child sessions for sub-agents, `summary` field for diffs, `revert` cursor for undo,
  ascending IDs (`ses_…`, `msg_…`, `prt_…`).
- **Cold archive** (rollouts) → JSONL per session in `~/.tdw/sessions/<id>/rollout.jsonl`,
  Codex-style. Cheap to replay, crash-safe, easy to grep.
- **Compaction** — Codex's "Memento" approach: cap user-message tokens at ~20k, generate LLM
  summary, replace history with `[system context] + [summary] + [recent messages]`. Triggered
  automatically by token-budget breach (`Op::Compact`) or manually.

For the planned `knowledge-graph-and-tags` work, sessions form one graph layer; provider
ingest runs form another; UDF registrations form a third — link them by `session_id` in SQLite
and you get cross-session queries ("which ingest job did this query depend on?") for free.

---

## 7. Config — layered with explicit semantics

**Pattern source:** opencode's eight-layer merge with concat-array semantics.

Use `figment` (0.10, 1.8M DL/mo) as the merge engine. Layers (TDW-flavored):

```
1. ~/.tdw/config.toml                      (user defaults)
2. $TDW_CONFIG (path)                      (env-pointed override file)
3. <project-root>/.tdw/config.toml         (project config)
4. <project-root>/.tdw/{providers,udfs,hooks}/*.toml   (split-file config)
5. $TDW_CONFIG_CONTENT (inline JSON)       (env-inline override)
6. CLI flags                               (final word)
```

**Concat-don't-replace** for: `providers`, `hooks`, `mcp_servers`, `instructions` (any list
that's a set of plugins, not a singleton choice).

**Substitutions:** `{env:FOO}` and `{file:./path}` resolved post-deserialize. Borrow this
verbatim from opencode — it's the right ergonomic for secrets.

**Strict schema validation** like Codex's `config/strict_config.rs` + emit JSON Schema (via
`schemars`) so editors get autocomplete. opencode's docs explicitly callout that JSON Schema
generation drove editor adoption — the same applies here.

---

## 8. LLM provider abstraction

**The trap:** every multi-provider Rust crate is opinionated. `genai` is the closest to right
but still beta. `rig` is well-funded but its `Agent`/`Completion`/`Tool` traits leak through.
`async-openai` is rock-solid but OpenAI-only.

**Recommendation:** define `crates/tdw-llm/` with a ~200-line `trait LanguageModel`:

```rust
#[async_trait]
pub trait LanguageModel: Send + Sync {
    fn id(&self) -> &ModelId;
    fn capabilities(&self) -> ModelCapabilities;
    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<StreamPart>>>;
}
```

Reference impls under `tdw-llm-anthropic` (Messages API) and `tdw-llm-openai-compat`
(absorbs OpenAI, Azure, vLLM, LM Studio, Ollama, Groq, DeepSeek, xAI, most local providers
via base-url swap). Pull model metadata from **`models.dev`** at startup (opencode does this;
it means new model releases don't require a TDW release).

This is the one place where lifting an external library wholesale is the wrong move — the
provider zoo is too unstable and your needs (`StreamPart` shape, tool-call envelope, cost
accounting) will diverge.

---

## 9. Repo-map / code intelligence — punt with intent

**Honest gap.** There is no `Aider::RepoMap` equivalent in Rust. You have to build it from
`tree-sitter` + `petgraph` + `tiktoken-rs` + per-language `.scm` queries.

For TDW specifically, **the codebase isn't the thing being mapped** — the **data warehouse
metadata is**. So the repo-map analogy is:

- **Tables / columns / lineage** = the file/symbol/reference graph in Aider.
- **Personalized PageRank on lineage** = "given the current query, rank tables by relevance."
- **Token-budgeted shrink loop** = "fit the schema preamble into the context window."

This is a **far better fit** for TDW than the literal code-repo-map. Make
`crates/tdw-knowledge/` own this — it satisfies the `knowledge-graph-and-tags.md` plan and
delivers Aider's killer feature in a domain-appropriate form.

Crates: `petgraph` (has `page_rank` directly), `tantivy` for keyword search over column
descriptions, `lancedb` for embedded vector search over column docstrings, `arrow-rs` for
sampling row distributions, `rusqlite`/`sqlx` for the catalog itself.

---

## 10. TUI — last, deliberately

`ratatui` (3.85M DL/mo, 0.30). The pattern from Codex's `tui/app.rs`:

- TUI is a **thin SSE/UDS consumer of `EventMsg`s**; it does not embed the agent.
- One unbounded `mpsc::UnboundedReceiver<AppEvent>` for internal UI events.
- One `tokio::select!` multiplexing: app events, agent events, terminal input,
  app-server events.
- Coalesce token-stream frames to 16-33ms ticks (ratatui is immediate-mode; redrawing on
  every token saturates stdout).
- `crossterm::event::EventStream` (async), not blocking `read()` — required for Ctrl-C
  during streaming.

Build the TUI **last**. The headless `tdw exec` path + `tdw mcp-server` are more important —
they make TDW agent-driveable from any frontend.

---

## 11. Mapping to the existing FinX-Plattform workspace

| Existing crate | Pattern that applies | New work |
|---|---|---|
| `tdw-provider-{alpaca,binance,fred,huggingface,polygon}` | Provider abstraction (opencode's `BUNDLED_PROVIDERS` map) | Unify behind a `trait DataProvider` in a new `tdw-provider-core` |
| `tdw-embed-{google,openai}` | Same as above; embeddings as a sub-type of provider | Move trait to `tdw-llm` |
| `tdw-udf-{external,js,python,wasm}` | Codex sandbox-per-crate + one adapter | New `tdw-sandbox` crate with `trait SandboxRuntime` |
| `tdw-storage-parquet` | Cold storage backbone for rollouts; arrow batch shape | Already fits |
| `tdw-rewrite` | SQL/plan rewrite — hooks can sit here as `PreQueryRewrite` event | Wire into `tdw-hooks` |
| `tdw-ml-registry` | Tool registry mirror (UDFs as ML models, models as tools) | Cross-link with `tdw-tools` registry |
| `tdw-fn-string` | Utilities — fine as-is | — |
| `tdw-test-utils` | Test substrate — extend with `protocol` replay helpers | Add rollout-replay test harness |

**New crates to add** (in dependency order):

1. `tdw-protocol` — `Op`, `EventMsg`, IDs, errors. Zero internal deps.
2. `tdw-config` — figment + schemars + substitution layer.
3. `tdw-hooks` — event bus, handler kinds, runtime.
4. `tdw-llm` + `tdw-llm-anthropic` + `tdw-llm-openai-compat`.
5. `tdw-sandbox` — UDF host adapter; existing `tdw-udf-*` crates plug in.
6. `tdw-tools` — `Tool` trait, `ToolRegistry`, `ToolRouter`, `ToolOrchestrator`.
7. `tdw-mcp-client` + `tdw-mcp-server` — `rmcp` wrappers.
8. `tdw-acp` — agent-client-protocol server.
9. `tdw-session` — sqlx + sqlite migrations; opencode's schema as template.
10. `tdw-rollout` — JSONL append-only logs.
11. `tdw-core` — the agent loop itself.
12. `tdw-app-server` + `tdw-app-client` — daemon split.
13. `tdw-knowledge` — Aider-style ranked retrieval over warehouse metadata.
14. `tdw-tui` (last) — ratatui frontend.
15. `tdw-exec` — headless CLI.

---

## 12. Workspace `Cargo.toml` additions (concrete)

```toml
[workspace.dependencies]
# already in tree most likely:
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = "0.3"

# new:
rmcp = { version = "1.7", features = ["client", "server", "schemars", "auth"] }
agent-client-protocol = "0.11"
schemars = "1"
figment = { version = "0.10", features = ["toml", "env", "json"] }
sqlx = { version = "0.9", features = ["runtime-tokio", "sqlite", "macros"] }
eventsource-client = "0.17"            # NOT reqwest-eventsource
ratatui = "0.30"
crossterm = "0.28"
petgraph = "0.6"
tree-sitter = "0.26"
wasmtime = { version = "45", features = ["component-model", "async"] }
pyo3 = { version = "0.28", optional = true }   # gate behind feature
rquickjs = { version = "0.11", features = ["async"] }
landlock = "0.4"                                 # linux only via cfg
tokio-util = { version = "0.7", features = ["rt"] }   # CancellationToken
async-channel = "2"                              # codex uses for exec deltas
utoipa = { version = "5", features = ["axum_extras"] }   # OpenAPI for app-server HTTP API
axum = "0.8"
similar = "2"                                   # rich diff for tool output
gix = { version = "0.66", default-features = false }    # pure-Rust git
lancedb = "0.10"                                # embedded vector store
tantivy = "0.22"                                # bm25 for keyword search
```

**Avoid:** `sled`, `llm-chain`, `reqwest-eventsource`, `deno_core`, community `anthropic-sdk`,
`kalosm`, `swiftide` (unless RAG pipeline is the explicit goal).

---

## 13. Sequencing — what to build first

Three weeks of pre-work that unlocks everything else:

**Week 1: Protocol + config + hooks skeleton**
- Land `tdw-protocol` with `Op`/`EventMsg` enums + IDs. No implementation, just types.
- Land `tdw-config` with figment + schemars + JSON Schema emission. Wire `~/.tdw/`,
  project `.tdw/`, env-substitution.
- Land `tdw-hooks` with `HookEvent` enum and `Handler` enum (command + http only at first).
  Defer `Prompt` and `Agent` handlers until `tdw-llm` exists.

**Week 2: LLM + MCP + tools skeleton**
- Land `tdw-llm` trait and `tdw-llm-anthropic` reference impl. Skip openai-compat first
  pass (most providers are OpenAI-compatible anyway via base URL swap).
- Land `tdw-mcp-client` and `tdw-mcp-server` over `rmcp`. The MCP server should expose
  one no-op tool initially so you can verify the wire.
- Land `tdw-tools` with `Tool` trait + `ToolRegistry`. No orchestrator yet; that comes with
  sandbox.

**Week 3: Sandbox adapter + session storage + app-server seam**
- Land `tdw-sandbox` with the `SandboxRuntime` trait; refactor existing `tdw-udf-wasm` as
  the reference impl.
- Land `tdw-session` (SQLite + migrations) and `tdw-rollout` (JSONL).
- Land `tdw-app-server` + `tdw-app-client` over UDS. At this point you can run a daemon and
  drive it from a script via `tdw-app-client`.

**Then** — and only then — `tdw-core` (the agent loop) and `tdw-knowledge` (warehouse-metadata
repo-map) and `tdw-tui` (ratatui frontend). Building those before the substrate locks in
the wrong abstractions; building them after is mechanical.

---

## 14. Patterns explicitly NOT lifted (and why)

| Pattern | Where | Why skipped |
|---|---|---|
| Effect.ts runtime | opencode | TS-only; no Rust analog without massive overhead. |
| Vercel AI SDK as the provider interface | opencode | No Rust equivalent with same coverage; pin to your own trait. |
| Bubble Tea / Lipgloss style | crush | `ratatui` is the answer for Rust; same patterns translate. |
| Bespoke JS plugin runtime | opencode | Use MCP as the single extension protocol. |
| LSP-as-code-intelligence | crush | Warehouse metadata, not code, is what TDW maps; LSP doesn't help. |
| `salsa` for incremental analysis | rust-analyzer | README still says "WORK IN PROGRESS"; reach for it only if needed. |
| Actor frameworks (`ractor`, `kameo`) | misc | Hand-rolled `tokio::select!` beats actors at <10 long-lived tasks. |
| Custom SSE parser | Codex's `client.rs` | Use `eventsource-client` unless provider envelopes force the hand. |
| `Coder` subclass per edit format | aider | Tool-call-based edits (Codex/Claude Code) are more robust for modern models. |

---

## 15. Open questions to resolve before week 1

1. **Where does query planning sit relative to the agent loop?** Is `SELECT` issued by the
   LLM the same code path as `SELECT` issued by `tdw exec`? Recommendation: yes, both flow
   through `Op::RunQuery` and emit `EventMsg::QueryRowDelta`. The LLM path just has the
   `ApprovalRequest` event in front.
2. **Cost accounting unit.** Codex tracks tokens; TDW must track tokens + bytes-scanned +
   query-cost. Bake all three into `EventMsg::TurnComplete { tokens, cost: Cost }` from day 1.
3. **MCP server identity.** Will `tdw-mcp-server` expose one big "agent" tool (Codex-style)
   or one tool per primitive (`tdw.query`, `tdw.ingest`, `tdw.register_udf`)? Recommendation:
   one tool per primitive. Easier for external agents to compose; LLM tool-routing works
   better with granular tools.
4. **Hook handler authentication.** If a hook is an HTTP POST, what stops a malicious config
   from posting prompts to attacker-controlled URLs? Bind hook HTTP handlers to allowlisted
   hosts at config-load time. Codex doesn't solve this; we should.

---

## References (source paths to study before implementing)

- Codex protocol: `codex-rs/protocol/src/protocol.rs`
- Codex tool stack: `codex-rs/core/src/tools/{registry,router,orchestrator}.rs`
- Codex hook engine: `codex-rs/hooks/src/{engine,events,declarations}.rs`
- Codex sandboxing: `codex-rs/core/src/{landlock,windows_sandbox,sandboxing}.rs`
- Codex compaction: `codex-rs/core/src/compact.rs::run_compact_task_inner_impl`
- Codex MCP server: `codex-rs/mcp-server/src/message_processor.rs`
- Codex app-server seam: `codex-rs/app-server*` (four crates)
- opencode session schema: `packages/opencode/src/session/session.sql.ts`
- opencode permission service: `packages/opencode/src/permission/index.ts`
- opencode bus: `packages/opencode/src/bus/{index.ts,bus-event.ts}`
- opencode config layering: `packages/opencode/src/config/config.ts`
- goose deps: root `Cargo.toml` (`rmcp` + `agent-client-protocol`)
- aider repo-map algorithm: `aider/repomap.py::RepoMap`
- Claude Code hook matrix: `code.claude.com/docs/en/hooks`
