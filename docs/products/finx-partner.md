# FinX Partner — Design Spec (partner-system W1)

> **Status:** design + capability spec (W1). Read-only architecture pass; no Rust
> changed in this wave. Grounds the W2–W6 build in the crates that already exist.
>
> **Vision (from `.plans/partner-system-plan.md`):** turn FinX — the data
> warehouse (268 routes), OpenBB parity, and the knowledge/learning system —
> into one **autonomous, learning, human-in-the-loop partner**: more useful, easier
> to use, deeply wired to the learning loop, acting autonomously *within gates*,
> with the human kept in the loop as a collaborator.

## 0. Two owner directives that frame this spec

1. **Primary surface = "all three, shared core."** Build the Partner Core **once**
   as a surface-agnostic module and expose it equally on (a) MCP, (b) the OpenBB
   Workspace copilot (`tdw-openbb-agent` `/v1/query` + widgets), and (c) CLI/TUI.
   Each surface is a *thin adapter*. The design explicitly forbids per-surface
   logic duplication. See §1 and §6.

2. **Autonomy default = "fully autonomous, audit-only."** The partner acts
   autonomously **inside the existing eval/trust/B9 gates**; the human reviews
   **after the fact** via a legible audit trail + digest, with one-gesture
   undo/correct that feeds learning — **not** a pre-approval inbox. Reversibility
   (governed forgetting / cold-plane retirement) + the eval gates are the safety
   net. Only genuinely low-confidence / high-impact / **irreversible** actions
   escalate to a wait-for-human proposal. This reframes W5 from "approvals inbox"
   to "audit & undo surface." See §4.

These are product constraints, not engineering preferences; the rest of the spec
is built to satisfy them with maximum reuse.

---

## 1. Partner Core — the conversational front door

### 1.1 Decision: a new `tdw-partner` crate, NOT a fork of `tdw-openbb-agent`, NOT a method on `KnowledgeRuntime`

Three candidates were assessed against the actual code:

| Option | Verdict |
|---|---|
| **Extend `tdw-openbb-agent`** | ❌ Wrong layer. `tdw-openbb-agent` is, by its own crate doc (`crates/tdw-openbb-agent/src/lib.rs:36`), *"pure, I/O-free… performs **no I/O** and enforces **no policy**."* Its `answer()` (`src/drive.rs:112`) only maps a `QueryRequest` + a `StreamingLanguageModel` into ordered `SseEvent`s. It is the OpenBB **wire contract**, not an orchestrator. Adding routing/KG/memory there would pollute a clean-room protocol mapper and couple core logic to one surface. |
| **Add `answer/route` methods to `KnowledgeRuntime`** | ❌ Overloads a crate that already does a lot (`crates/tdw-knowledge/src/runtime.rs` — 1300+ lines of builder + feeds + freshness + proposals). The runtime is the *knowledge* engine; Partner Core also needs the **data dispatcher** (`tdw-service-api`) and **analytics compute**, which `tdw-knowledge` must not depend on (layering inversion). |
| **New `tdw-partner` crate** | ✅ **Chosen.** A thin orchestration crate that *composes* `KnowledgeRuntime`, the dispatcher, and the catalog behind one surface-agnostic entry. Owns nothing the others own; adds only the route-resolution + turn-assembly glue. Each of the 3 surfaces is a thin adapter over it. |

**Why a crate and not just a module in `tdw-service-api`:** `tdw-service-api` is the
data-plane (`dispatcher.rs` is 4900+ lines). Partner Core needs `tdw-service-api`
*and* `tdw-knowledge` *and* `tdw-endpoint-catalog`; putting it inside the dispatcher
crate would make `tdw-knowledge` ↔ `tdw-service-api` a bidirectional dependency.
A leaf crate `tdw-partner` that depends on all three (and is depended on by the
adapters) keeps the DAG clean.

### 1.2 The one entry: `PartnerCore::turn`

```rust
// crates/tdw-partner/src/lib.rs   (new)
pub struct PartnerCore {
    knowledge: Arc<KnowledgeRuntime>,         // tdw-knowledge
    dispatch:  Arc<dyn DataPlane>,            // thin trait over tdw-service-api dispatch_op/rest_fetch_data
    model:     Arc<dyn StreamingLanguageModel>, // tdw-llm
    // route-resolution config; catalog() is consulted, not stored
}

pub struct PartnerTurn {
    pub principal: Principal,                 // §6 — session/user/agent identity + trust
    pub utterance: String,
    pub context: TurnContext,                 // prior turns, attached widgets (Workspace), cwd (CLI)
}

pub enum PartnerEvent {                       // surface-agnostic; maps 1:1 to SseEvent / MCP / CLI lines
    Reasoning(String),
    Answer(String),                           // streamed fragment
    Citation(Provenance),                     // route ids + KG node ids + as_of
    DataRequest(WidgetDataRequest),           // Workspace two-request leg only
    Action(ActionRecord),                     // audit record of an autonomous act (§4)
}

impl PartnerCore {
    /// THE single decision point for a partner turn. Surface-agnostic; emits an
    /// ordered stream of PartnerEvent. Adapters render these to their transport.
    pub async fn turn(&self, turn: PartnerTurn, sink: &mut dyn FnMut(PartnerEvent)) -> Result<TurnOutcome>;
}
```

### 1.3 The turn flow (NL → route → memory → grounded answer → write-back)

```
utterance + principal
   │
   ▼
[1] RESOLVE  ── classify intent + select route(s)
   │   • data?      → tdw_endpoint_catalog::lookup / is_valid_route  (catalog() = the 268)
   │   • knowledge? → tdw.kg.search / tdw.kg.answer / tdw.kg.why
   │   • analytics? → a compute route in the catalog
   │   (LLM tool-selection bounded by the catalog — NOT free-form; see §1.4)
   ▼
[2] CONTEXT  ── memory-aware fetch, trust-filtered, point-in-time
   │   • KnowledgeRuntime::search(query, top_k)          (lib.rs:357)
   │   • tdw.kg.answer / tdw.kg.why for grounded facts + provenance
   │   • episodic recall (prior turns for this principal; §6)
   │   • trust filter from the principal's trust context (§6)
   ▼
[3] EXECUTE  ── fetch data / run compute
   │   • DataPlane::fetch(route, params) → dispatch_op / rest_fetch_data (dispatcher.rs:68,475)
   │   • Workspace surface: if widgets are primary & no tool result yet,
   │     emit DataRequest and close the leg (reuse decision::needs_widget_data)
   ▼
[4] ANSWER   ── assemble prompt from context+data, drive model.complete_streaming,
   │            stream Answer fragments + a closing Citation (route ids + KG nodes + as_of)
   ▼
[5] WRITE-BACK (always, autonomous within gates — §3/§4)
       • episodic memory of the turn            → tdw.kg.remember path
       • candidate finding(s)                   → tdw.kg.finding
       • feedback hook (thumbs / correction)    → tdw.kg.feedback → self_tune/lessons
       • any KG mutation → Proposal (gated)     → ProposalQueue::submit (proposals.rs:284)
```

### 1.4 Exact seams it composes (reuse, do not re-implement)

| Step | Reused function (file:line) |
|---|---|
| Route validity / lookup | `tdw_endpoint_catalog::catalog()`, `::lookup(route)`, `::is_valid_route(route)` (`crates/tdw-endpoint-catalog/src/lib.rs:107,134,145`) |
| Data fetch | `tdw_service_api::dispatcher::dispatch_op(state, env)`, `::rest_fetch_data(...)` (`dispatcher.rs:68,475`) |
| KG retrieval | `KnowledgeRuntime::search(query, top_k)` (`tdw-knowledge/src/lib.rs:357`); MCP `tdw.kg.answer/why/search` handlers (`knowledge_answer_tools.rs`, `knowledge_explain_tools.rs`) |
| Workspace widget leg | `tdw_openbb_agent::needs_widget_data(request)` + `SseEvent` builders (`decision.rs`, `event.rs`) |
| Answer streaming | `tdw_openbb_agent::assemble_chat_request` + `model.complete_streaming` (the pattern in `drive.rs:74`) |
| KG write (gated) | `ProposalQueue::submit(agent_id, adaptivity, kind, graph, tags, now)` (`proposals.rs:284`) |
| Episodic / finding / feedback | MCP handlers behind `tdw.kg.remember` / `tdw.kg.finding` / `tdw.kg.feedback` |

**Adversarial note — the simplest thing that works:** `PartnerCore::turn` is a
*sequencer*, mirroring the existing `tdw_openbb_agent::answer` design (a pure
ordered-event emitter) but with I/O steps injected as `Arc<dyn>` ports
(`DataPlane`, `KnowledgeRuntime`, model). **No new agent loop, no planner DSL, no
graph executor.** Route resolution is LLM tool-selection *bounded by `catalog()`*
— the model picks from the 268 + the `tdw.kg.*` verbs, and an invalid pick is
rejected by `is_valid_route` before any I/O. This is the anti-over-engineering line:
we are wiring, not building a new brain.

### 1.5 Collapsing the tool-soup

The MCP surface exposes **49 `tdw.*` tools** today (`crates/tdw-mcp/src`, e.g. 40
`tdw.kg.*` + data/route/widget/tag tools). Partner Core presents **one** verb to
the human — "ask FinX anything" — and internally dispatches to the right tool.
The 49 stay registered (power users + agents keep them), but the default surface
is the single `tdw.partner.ask` (§5 onboarding).

---

## 2. Proactive layer — one brief + nudge stream

### 2.1 What exists, and the gap

The proactive *primitives* are **pure compute**, with **no scheduler of their own**:

- `tdw-alerts`: `PriceAlert`, `NewAlert`, `InMemoryAlertStore` / `PgAlertStore`
  (`crates/tdw-alerts/src/lib.rs:65,122,228,333`).
- `tdw-alert-evaluator`: `AlertEvalDeps` + evaluation (`src/lib.rs:64`).
- `tdw-watchlist-compose`: pure `compose/normalize/dedup/is_valid` over
  `WatchlistEntry` (`src/lib.rs:129,148,208,228`).
- `tdw-news-compose`: pure `compose/compose_for_symbols` over `Article`
  (`src/lib.rs:266,308`).
- Knowledge-side signals already exist as MCP verbs: `tdw.kg.thesis_health`,
  `tdw.kg.questions` (open questions), `tdw.kg.digest` (staleness digest),
  `tdw.kg.diff` (contradictions/changes).

The **scheduler already exists**: `tdw-cron` — `ScheduleRegistry`,
`spawn_cron_scheduler`, `due_triggers`, `build_job`, `cron_tick()`
(`crates/tdw-cron/src/lib.rs:184,332,237,275,399`). And `KnowledgeRuntime` already
carries a **feed tick** with freshness cells (`with_feed_freshness`,
`feed_freshness_cells`, `with_watchlist_freshness`, `with_questions_freshness`,
`with_distillation_freshness` — `runtime.rs:502,509,520,547,666`).

**Gap:** no single type that *unifies* these into one "what changed / what I
noticed / what needs you" stream.

### 2.2 Design: a `Nudge` model + a `brief` assembler, scheduled by `tdw-cron`

```rust
// crates/tdw-partner/src/proactive.rs  (new, in the Partner crate — reuses primitives)
pub struct Nudge {
    pub id: String,
    pub principal: Principal,        // who it's for (§6)
    pub kind: NudgeKind,             // AlertFired | ThesisHealth | OpenQuestion | Contradiction | Staleness | ActionTaken
    pub severity: Severity,
    pub headline: String,
    pub provenance: Provenance,      // route ids / KG node ids / alert id that produced it
    pub created_at: String,
    pub dismissed: Option<Dismissal>,// dismissal feeds learning (§2.3)
}

/// Assemble the morning brief = the union of the unified signal sources,
/// ranked by severity × trust × recency. Pure given its inputs.
pub fn build_brief(signals: BriefInputs, principal: &Principal) -> Vec<Nudge>;
```

**Scheduler choice:** **`tdw-cron`**, not a bespoke loop. Register two triggers:
- a daily "morning brief" `ScheduledTrigger` → `build_job` → fans into
  alert eval + `tdw.kg.digest` + `tdw.kg.thesis_health` + `tdw.kg.questions` +
  `tdw.kg.diff`, then `build_brief`.
- the existing `KnowledgeFeed` tick stays as the **event-driven** path (a fired
  alert or a fresh contradiction emits a `Nudge` immediately).

Rationale: `tdw-cron` already does cron parsing, due-trigger selection, and job
spawning with a worker queue; reusing it avoids a second scheduling concept. The
feed tick handles low-latency event nudges; cron handles the scheduled brief.

### 2.3 Dismissal feeds learning

A `Dismissal` (or a "useful" tap) is a feedback signal: it routes through the same
`tdw.kg.feedback` path into `self_tune` (`crates/tdw-knowledge/src/self_tune.rs` —
`propose_candidate`, `decide`, `SelfTuneLog::record`) and `lessons`
(`crates/tdw-knowledge/src/lessons.rs` — `Lesson`, `LessonState`). Repeatedly
dismissing a nudge kind lowers its rank / suppresses it (a tuned parameter,
B9-gated like every other behavior change). This is the W4 closure for the
proactive lane.

---

## 3. Learning-loop wiring — where an interaction becomes behavior

The loop is **use → learn → improve → proactively help**, and every behavior change
is **B9 / eval-gated**. Concretely:

```
interaction (a turn or a nudge tap)
   │  feedback hook (§1.3 step 5)
   ▼
tdw.kg.feedback  ──►  self_tune::propose_candidate / decide   (self_tune.rs:139,215)
   │                  lessons: Lesson / LessonState           (lessons.rs:110,230)
   │                  induction: InductionEngine::run_cycle /
   │                             induce_candidate → CandidateRule (induction.rs:287,398,147)
   ▼
B9 / Adaptivity GATE  (the universal safety net)
   • Adaptivity enum (tdw-taxonomy/src/facets.rs:59): None < Configured < Learning < SelfModifying
   • FEEDBACK_MIN_ADAPTIVITY = Adaptivity::Learning  (tdw-agent/src/base.rs:93)
   • ensure_adaptive_for_feedback(meta)              (tdw-agent/src/base.rs:120)
   • ProposalQueue::submit refuses below Learning    (proposals.rs:297)
   • eval freshness / promotion threshold            (runtime.rs with_eval_freshness:406;
                                                       ProposalQueue::with_ready_threshold:250)
   ▼
PROMOTE (only past the gate)
   • CandidateRule::is_promoted (induction.rs:167) → InferEngine::hot_reload (infer.rs:302)
   • ProposalQueue::promote_for_agent / materialize_ready (proposals.rs:356,439)
   • KnowledgeRuntime::update_versions(rules_version, infer_version) (runtime.rs:260)
   ▼
BEHAVIOR
   • Partner Core re-reads via KnowledgeRuntime::versions() (runtime.rs:292) +
     the AdaptivityResolver (with_adaptivity_resolver:320 / adaptivity_resolver():327)
   • trust-dial + learned preferences shape step [2] CONTEXT and step [1] RESOLVE
   • proactive layer re-ranks nudges from the tuned parameters
```

**The one rule:** no behavior change reaches Partner Core or the nudge stream
except by bumping a version (`update_versions`) or materializing a proposal — both
of which are gated. The `KnowledgeRuntime` already exposes the version seam and the
`AdaptivityResolver`, so Partner Core consumes learning **by reading the runtime it
already holds** — no new wiring beyond reading `versions()` per turn.

**Trust-dial → retrieval:** the principal's trust context (§6) is passed into step
[2] so that low-trust knowledge is down-weighted/filtered at retrieval, and the
autonomy threshold in §4 is read from the same dial. Trust *rises* on accepted
actions and *falls* on undo/correction (§4.3) — the trust-dial is itself a tuned,
gated parameter.

**Walk-forward eval (W4 done-condition):** `tdw-eval-runner` drives a replay where
the same query stream is answered before/after a learning epoch; the metric is
rising grounded-answer quality + routing accuracy + nudge acceptance. This proves
"gets more useful with use" rather than asserting it.

---

## 4. Autonomy & the audit surface (reframed W5) — "fully autonomous, audit-only"

### 4.1 Default: act within gates, review after the fact

Per directive 2, the partner **does not** queue routine actions for pre-approval.
It acts autonomously whenever the action is **inside the gates** (Adaptivity ≥
Learning, eval/threshold satisfied) **and reversible**. The human reviews **after
the fact**. The safety net is **reversibility + eval gates**, not a human bottleneck.

This is already supported by the substrate:

- **Reversibility is first-class.** `ProposalKind::Forget` (`proposals.rs:148`)
  retires a fact to the **COLD plane** by closing its validity window — *the
  historical record is preserved and the move is fully reversible* via
  `forgetting::recall_cold_edge` (`crates/tdw-knowledge/src/forgetting.rs:475`).
  Every autonomous KG write therefore has a defined undo.
- **Materialization re-validates.** `materialize_ready` re-checks at write time and
  rejects stale proposals (`MaterializeReport.rejected_at_materialize`,
  `proposals.rs:214`) — a learned change can never clobber a fact asserted in the
  meantime.

### 4.2 The audit/undo surface (not an inbox)

```rust
// crates/tdw-partner/src/audit.rs  (new — a VIEW over existing records, not a new store)
pub struct ActionRecord {
    pub id: String,
    pub principal: Principal,
    pub what: ActionKind,        // KgWrite(Proposal) | Forget | ParamTune | RuledPromote | NudgeSent
    pub why: Provenance,         // tdw.kg.why chain + the feedback/eval that triggered it
    pub confidence: f64,
    pub reversible: bool,        // true for KG writes (Forget exists) + param tunes (re-tune)
    pub status: ActionStatus,    // AutoAccepted | AwaitingHuman | Undone | Corrected
}

pub fn audit_feed(filter: AuditFilter) -> Vec<ActionRecord>;   // the "what I did + why" stream
pub fn undo(action_id: &str) -> Result<()>;                    // one-gesture reverse
pub fn correct(action_id: &str, correction: Correction) -> Result<()>; // reverse + feedback
```

The audit feed is a **projection** over records that already exist — the
`ProposalQueue` history (`Proposal.history`, `proposals.rs:177`), the
`SelfTuneLog` (`self_tune.rs:283`), the `LessonAudit` (`lessons.rs:245`), and
`tdw.kg.why`. It introduces **no new system of record**; it reads and renders.

- **Undo** = `recall_cold_edge` for a `Forget`, or a `Forget` proposal for an
  added edge, or a re-tune for a parameter. Always reversible-by-construction.
- **Correct** = undo **+** a `tdw.kg.feedback` signal, so the correction *trains*
  the system (trust down for that action's source, a `Lesson` recorded).

### 4.3 When autonomy escalates to wait-for-human

A small, explicit predicate (read from the trust-dial + a config) escalates to an
`AwaitingHuman` proposal instead of auto-accepting:

- **low confidence** (below the principal's trust threshold for that action kind),
- **high impact** (touches a node above an importance threshold, e.g. a core
  thesis), or
- **irreversible** (no defined undo — should be rare by design; flagged loudly).

These land in the *same* audit feed with `status = AwaitingHuman`, so there is **one
surface** for "what I did" and "what I'm waiting on you for" — not two. Default
posture is auto-accept-within-gates; the escalation set is the exception.

### 4.4 Why audit-only is safe here

The combination that makes after-the-fact review trustworthy: (1) every write is
gated (≥ Learning + eval threshold), (2) every write is reversible (Forget /
cold-plane / re-tune), (3) materialization re-validates against the live graph,
and (4) the audit feed renders `why` (the `tdw.kg.why` provenance chain) for every
action. The human's leverage is **undo + correct**, which is strictly more
informative to the learning loop than pre-approval (a rejection pre-write teaches
less than a correction post-write tied to an observed outcome).

### 4.5 Surfaces for the audit feed

One read model, three thin renderers (mirrors §1/§6): an MCP tool
(`tdw.partner.audit` + `tdw.partner.undo`), a REST endpoint, a CLI command, and a
**Workspace widget** (the existing widget surface in `tdw-openbb-agent`/Workspace).
The existing `tdw.kg.proposals` / `tdw.kg.dismiss` MCP verbs are folded in as the
KG-write slice of this feed.

---

## 5. Onboarding / ease — zero-to-partner

**Goal:** connect context → first watchlist/thesis → see the loop, in < 10 min,
without meeting 49 tools.

- **One default verb.** New users see `tdw.partner.ask` (Partner Core, §1) and
  `tdw.partner.brief` (proactive, §2). The other 47 `tdw.*` tools become
  "advanced" — registered but not surfaced by default. A progressive-disclosure
  manifest: `default_manifest` (`tdw-openbb-agent/src/manifest.rs`) advertises the
  partner agent; the full tool list is opt-in.
- **Guided first run.** A scripted sequence (a `tdw-workflow-engine` workflow):
  (1) connect a data context (pick a few symbols → seeds a `WatchlistEntry` via
  `watchlist-compose`), (2) ask one question (exercises Partner Core end-to-end →
  writes the first episodic memory + finding), (3) state one thesis
  (`tdw.kg.thesis`), (4) show the first brief (§2) and the audit feed (§4) so the
  loop is *visible* immediately.
- **The "intelligent few":** `ask`, `brief`, `audit`, plus `watch`/`thesis` for
  setup. Everything else is reachable through `ask` (which dispatches internally).

---

## 6. Shared persona / memory / trust seam

Built **once** in `tdw-partner`, consumed by all surfaces (directive 1).

```rust
// crates/tdw-partner/src/principal.rs (new)
pub struct Principal {
    pub session_id: String,     // tdw-session SessionRecord (session/src/lib.rs:31)
    pub user_id: Option<String>,// KnowledgeRuntime::with_user_id / bound_user_id (runtime.rs:366,373)
    pub agent_id: String,       // KnowledgeRuntime::with_agent_id / bound_agent_id (runtime.rs:350,357)
    pub kg_namespace: String,   // with_graph_name (runtime.rs:238) — per-principal KG scope
    pub trust: TrustContext,    // the trust-dial + Adaptivity for this principal
}
```

| Concern | Shared across surfaces | Per-surface |
|---|---|---|
| Session principal | ✅ `Principal` (one identity per session) | the transport that *establishes* it (MCP auth / Workspace token / CLI env) |
| KG namespace | ✅ `kg_namespace` + `agent_id`/`user_id` binding on the runtime | — |
| Trust context | ✅ one trust-dial drives retrieval filtering (§3) + autonomy threshold (§4) | — |
| Episodic memory | ✅ written once per turn, recalled by any surface | — |
| Persona / answer style | ✅ one prompt-assembly path | rendering (SSE vs MCP JSON vs CLI text) |
| Transport mechanics | — | listener/CORS/auth (Workspace), stdio (MCP), TTY (CLI) |

The three adapters are **thin**: each maps `PartnerEvent` (§1.2) to its wire format
and `Principal` to its auth context. **No routing, retrieval, learning, or autonomy
logic lives in any adapter** — that is the explicit anti-duplication invariant. A
review test (W6) asserts the adapters contain no `tdw_endpoint_catalog`,
`ProposalQueue`, or `self_tune` calls — only `PartnerCore` does.

---

## 7. Wave breakdown (W2–W6), PR-sized, crate-by-crate

Legend: **new** = new file/crate; **wire** = compose existing; each task names its
eval/gate.

### W2 — Partner Core (the shared conversational front door)

| # | Task | Crates / files | Eval / gate |
|---|---|---|---|
| W2.1 | Scaffold `tdw-partner` crate; `PartnerCore`, `PartnerTurn`, `PartnerEvent`, `Principal`, `Provenance` types | **new** `crates/tdw-partner/{Cargo.toml,src/lib.rs,principal.rs}`; deps `tdw-knowledge`, `tdw-service-api`, `tdw-endpoint-catalog`, `tdw-llm` | compiles; type-design review |
| W2.2 | `DataPlane` port + impl over the dispatcher | **new** `src/dataplane.rs`; **wire** `dispatcher::dispatch_op`/`rest_fetch_data` | unit: route → fetch round-trip |
| W2.3 | Route resolution bounded by `catalog()` (LLM tool-select + `is_valid_route` guard) | **new** `src/resolve.rs`; **wire** `endpoint-catalog::{catalog,lookup,is_valid_route}` + `tdw.kg.*` verb set | **routing-accuracy eval** (golden query→route set) |
| W2.4 | `turn()` sequencer: resolve→context→execute→answer→write-back; reuse the `answer()` event pattern | `src/lib.rs`; **wire** `KnowledgeRuntime::search`, `tdw.kg.answer/why`, `assemble_chat_request`, `complete_streaming` | **grounded-answer-quality eval** (offline `StubLanguageModel`) |
| W2.5 | Write-back: episodic + finding + feedback; KG writes via `ProposalQueue::submit` | **wire** `tdw.kg.remember/finding/feedback`, `proposals::submit` | gate: submit refuses < Learning (assert) |
| W2.6 | **MCP adapter** `tdw.partner.ask` | **wire** `crates/tdw-mcp` (new tool module) | e2e: ask → streamed answer + citation |
| W2.7 | **Workspace adapter**: route `PartnerCore` through the agent bridge instead of bare `answer()` | `crates/tdw-service-api/src/agent_bridge.rs` (`AgentBridgeState` gains a `PartnerCore`); reuse `needs_widget_data` two-leg | e2e: `agent_route_e2e.rs`-style golden SSE |
| W2.8 | **CLI/TUI adapter** | the CLI crate; render `PartnerEvent` to TTY | smoke: `ask` from CLI |

> **Highest-leverage in W2: W2.7.** `agent_bridge.rs` already delegates the whole
> Workspace turn to `tdw_openbb_agent::answer(&request, model)`. Swapping that one
> call for `partner.turn(...)` lights up the Workspace surface for free — this is
> the single seam that makes "shared core, thin adapter" real.

### W3 — Proactive layer (brief + nudges)

| # | Task | Crates / files | Eval / gate |
|---|---|---|---|
| W3.1 | `Nudge` model + `build_brief` assembler | **new** `crates/tdw-partner/src/proactive.rs` | unit: ranking determinism |
| W3.2 | Unify signal sources into `BriefInputs` | **wire** `tdw-alerts`/`tdw-alert-evaluator`, `tdw.kg.{digest,thesis_health,questions,diff}` | golden brief from fixed inputs |
| W3.3 | Schedule via `tdw-cron`: daily brief trigger + worker job | **wire** `cron::{ScheduleRegistry,spawn_cron_scheduler,build_job}` | scheduled-fire test (pinned clock) |
| W3.4 | Event-driven nudges off the `KnowledgeFeed` tick | **wire** `runtime` feed-freshness cells | latency test: contradiction → nudge |
| W3.5 | Dismissal → feedback → `self_tune`/`lessons` | **wire** `tdw.kg.feedback`, `self_tune`, `lessons` | gate: dismissal re-rank is B9-gated |
| W3.6 | Surfaces: `tdw.partner.brief` (MCP) + CLI; Workspace brief widget | adapters | e2e per surface |

> **Highest-leverage in W3: W3.3 (reuse `tdw-cron`)** — do **not** build a new
> scheduler. The whole proactive lane is "fan existing signals into one ranked list
> on a cron trigger."

### W4 — Learning-loop closure (make it real + measured)

| # | Task | Crates / files | Eval / gate |
|---|---|---|---|
| W4.1 | Partner Core reads `versions()` + `AdaptivityResolver` per turn so promotions take effect | `tdw-partner`; **wire** `runtime::{versions,adaptivity_resolver}` | unit: bumped version changes behavior |
| W4.2 | Trust-dial → retrieval filter in step [2]; learned prefs shape step [1] | `tdw-partner`; **wire** trust context + `self_tune` outputs | eval: trust-filtered retrieval |
| W4.3 | Induced rules feed routing/answers via `InferEngine::hot_reload` | **wire** `induction::run_cycle`, `infer::hot_reload` | gate: `CandidateRule::is_promoted` only past B9 |
| W4.4 | **Walk-forward eval** harness proving rising usefulness | **wire** `tdw-eval-runner` replay | the W4 done-condition metric |

> **All W4 changes are B9-gated by construction** — Partner Core only *reads* the
> gated runtime; it never mutates behavior directly.

### W5 — Audit & undo surface (reframed: NOT a pre-approval inbox)

| # | Task | Crates / files | Eval / gate |
|---|---|---|---|
| W5.1 | `ActionRecord` + `audit_feed` projection over existing records | **new** `crates/tdw-partner/src/audit.rs`; **wire** `Proposal.history`, `SelfTuneLog`, `LessonAudit`, `tdw.kg.why` | unit: feed renders `why` for every action |
| W5.2 | Auto-accept-within-gates default + escalation predicate (low-conf/high-impact/irreversible) | `src/audit.rs`; **wire** trust-dial + config | gate test: routine acts auto-accept; flagged acts → `AwaitingHuman` |
| W5.3 | `undo` (reuse `recall_cold_edge` / inverse `Forget` / re-tune) | **wire** `forgetting::recall_cold_edge`, `proposals` | reversibility test: write→undo→state restored |
| W5.4 | `correct` = undo + `tdw.kg.feedback` (trains the system) | **wire** feedback path | gate: correction lowers trust + records `Lesson` |
| W5.5 | Surfaces: `tdw.partner.audit`/`undo` (MCP) + REST + CLI + **Workspace audit widget**; fold in `tdw.kg.proposals`/`dismiss` | adapters | e2e per surface |

> **Highest-leverage in W5: W5.1+W5.3.** A read-only projection + a working undo,
> built on the *already-reversible* `Forget`/cold-plane machinery, is what makes
> "audit-only autonomy" safe. **No new system of record.**

### W6 — Cohesion, onboarding & release

| # | Task | Crates / files | Eval / gate |
|---|---|---|---|
| W6.1 | Anti-duplication test: adapters contain no core logic | adapters + a workspace test | review-gate: adapters only map events/principal |
| W6.2 | Guided "zero-to-partner" first run | **wire** `tdw-workflow-engine` workflow | < 10 min walkthrough passes |
| W6.3 | Progressive-disclosure manifest (default = `ask`/`brief`/`audit`) | `manifest.rs` + MCP registration | onboarding eval |
| W6.4 | Docs + final architect gate + partner release | `docs/products/finx-partner.md` (this) + release | architect sign-off |

---

## 8. The 2–3 highest-leverage moves (vs nice-to-haves)

1. **W2.7 — route the Workspace bridge through `PartnerCore`.** One-line-ish swap in
   `agent_bridge.rs` (`answer()` → `partner.turn()`) turns the existing copilot into
   the real partner and validates the shared-core/thin-adapter thesis immediately.
2. **W5.1+W5.3 — audit projection + undo on the existing `Forget`/cold-plane.** This
   is what makes "fully autonomous, audit-only" *safe* rather than reckless, and it
   reuses machinery that already exists (`recall_cold_edge`, `ProposalKind::Forget`).
   Without reversibility, audit-only autonomy is indefensible; with it, it's the
   better learning signal.
3. **W3.3 — schedule the brief on `tdw-cron`.** Unlocks the entire proactive lane by
   reusing the scheduler instead of inventing one; the primitives are already pure
   compute waiting for a trigger.

## 9. Recommended DROPs / de-scopes (anti-over-engineering)

- **DROP a pre-approval "approvals inbox."** Directive 2 makes it the wrong default;
  the audit/undo surface (W5) subsumes it. The `AwaitingHuman` escalation reuses the
  same feed — no separate inbox UI, store, or workflow.
- **DROP any new planner/agent-loop/orchestration DSL.** `PartnerCore::turn` is a
  sequencer composing existing ports; a graph executor would be over-engineering for
  resolve→fetch→answer→write-back.
- **DROP a new scheduler.** `tdw-cron` + the `KnowledgeFeed` tick cover scheduled and
  event-driven nudges respectively.
- **DROP a new proposal/record store for audit.** W5 is a *projection* over
  `Proposal.history` + `SelfTuneLog` + `LessonAudit` + `tdw.kg.why`. Adding a store
  would duplicate the system of record.
- **DROP a unified cross-subsystem `Proposal` super-type rewrite.** The KG
  `Proposal`/`ProposalQueue` is the one *write-gating* abstraction worth standardizing
  on; the other "candidate" types (`CandidateRule`, `ForgettingCandidate`,
  `Lesson`, self-tune records, `PendingApprovalRecord`) are **promotion/learning**
  artifacts with their own gates and need not collapse into one type. The audit feed
  (§4.2) unifies them at the *view* layer, not the *type* layer — much cheaper and
  non-invasive. Forcing one super-type across subsystems would be a large, risky
  refactor for no functional gain.

## 10. Open questions for W2 kickoff

- Does Partner Core's route-resolution LLM call reuse the daemon's existing
  `StreamingLanguageModel` credential gates (per `agent_bridge.rs` comments), or need
  a separate budget? (Lean: reuse.)
- Trust context shape: is the trust-dial already a stored per-(user,agent) value, or
  must W4.2 introduce its persistence? (Audit before W4.)
- Workspace widget framework for the audit/brief widgets — confirm the existing
  widget contract (`tdw.widgets.describe/list`) covers a custom partner widget.

---

*Grounded in: `tdw-openbb-agent` (drive/decision/event/manifest), `tdw-service-api`
(dispatcher, agent_bridge), `tdw-endpoint-catalog`, `tdw-knowledge` (runtime,
proposals, self_tune, lessons, forgetting, feeds), `tdw-induction`, `tdw-infer`,
`tdw-taxonomy` (Adaptivity), `tdw-agent` (base/adaptivity gate), `tdw-session`,
`tdw-cron`, `tdw-alerts`/`tdw-alert-evaluator`, `tdw-watchlist-compose`,
`tdw-news-compose`, `tdw-eval-runner`, and the 49 `tdw.*` MCP tools in `tdw-mcp`.*
