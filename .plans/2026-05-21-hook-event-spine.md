# FinX-Finance — Hook & Event Spine (Layer E)

**Project:** FinX-Finance (`C:\Users\ReyDa\FinX-Finance\`)
**Date:** 2026-05-21
**Mode:** `/omc-plan --direct` (extension; introduces new Phase 9, renumbers existing 9–13 to 10–14)
**Status:** Draft — load-bearing layer that several existing phases depend on
**Parent plans:**
- [`2026-05-21-rust-trading-data-warehouse.md`](./2026-05-21-rust-trading-data-warehouse.md) — core (Phases 0–6)
- [`2026-05-21-data-engineering-and-agent-schemas.md`](./2026-05-21-data-engineering-and-agent-schemas.md) — Layer A+B (Phases 7–8)
- [`2026-05-21-databend-surrealdb-feature-parity.md`](./2026-05-21-databend-surrealdb-feature-parity.md) — Layer C (was Phases 9–13; **renumbered to 10–14** by this plan)
- [`2026-05-21-connect-rust-buffa-evaluation.md`](./2026-05-21-connect-rust-buffa-evaluation.md) — evaluation only, no phase changes

---

## 1. Goal

Build a **bidirectional, actor-aware, typed hook & event spine** that:

1. **DB → Rust direction**: reacts to Postgres + ClickHouse change events (onInsert, onUpdate, onDelete, onSchemaChange, onCommit) and dispatches them to registered Rust hooks.
2. **Rust → DB direction**: typed Rust events emitted by other crates flow through the spine and produce durable, transactional DB writes via the outbox pattern.
3. **Differentiates actors**: every event carries `Actor = Agent | User | System | Worker | External`, propagated through `tokio::task_local!` + the event envelope, enforced through capability tokens.
4. **Distinguishes client vs server origin**: set at the process entry point (`Client` = external request, `Server` = internal task), used for recursive-event protection, audit semantics, and retry policy.
5. **Easy to manage / extend / configure**: declarative `#[hook]` attribute + compile-time registration (`linkme::distributed_slice`), runtime enable/disable via a `hooks` Postgres table, JSON-Schema-validated event types, WordPress-style explicit priority.
6. **Subsumes and replaces** existing plan elements: `tdw-stream`, `tdw-live`, `tdw-notify`, and the DEFINE EVENT half of `tdw-define` become thin adapters on this spine instead of separate primitives. Net code simplification across Phases 11–14.

**This layer is load-bearing.** It must ship before the previously-planned Phase 10 (Streams + Live Queries) and before Phase 14 (UDFs + Auth + DEFINE), because both consume it. Hence the renumbering.

---

## 2. RALPLAN-DR Summary

### Principles
1. **Two lanes, one envelope.** Hot path = in-process `tokio::sync::broadcast` (low-latency, lossy on slow subscribers). Durable path = Postgres outbox + RiverQueue (at-least-once, retried). Same `EventEnvelope<P>` shape on both.
2. **Actor identity is mandatory.** No anonymous events. Every emit carries an `Actor` discriminant; missing actor = compile error or runtime reject.
3. **Sync vs async hooks are declared, never inferred.** A hook either runs *inside* the originating transaction (can veto) or *after* commit (retried). The choice is fixed at registration; no "maybe async" middle ground.
4. **Recursive-event protection is structural, not advisory.** `depth: u8` in the envelope; `MAXDEPTH = 8`; a `(kind, primary_key)` set in `task_local!` prevents re-firing the same logical event within one correlation chain.
5. **Specialist primitives only.** No Kafka, no NATS, no Debezium, no actix actor framework. Just `tokio::sync::broadcast` + `supabase/etl` (a.k.a. `pg_replicate`) + Postgres outbox + RiverQueue. Everything else earns its weight or it stays out.

### Decision Drivers (top 3)
1. **Bidirectional with one mental model.** Without unification, the user ends up writing different code for "react to DB change" vs "emit event from crate." The spine collapses both into "publish to the bus / subscribe to the bus."
2. **Single-machine personal scope.** Kafka/NATS/Debezium-class infra is the wrong shape; ops weight dominates value.
3. **Existing Layer C surface (streams, live queries, DEFINE EVENT, notifications) is fragmented.** Building the spine first means those four planned features become ~50% less code each.

### Viable Options

#### Option A — Two-lane (broadcast + outbox) with declarative hooks *(chosen)*
- **Pros**: matches the two research recommendations exactly; low-latency hot path; durable cold path; idempotent via UUIDv7; ~1500 LOC of Rust; pure-Rust deps; sync+async hook traits typed; recursive guard cheap.
- **Cons**: two lanes = two failure modes to debug; outbox publisher is a process-singleton SPOF (mitigated by RiverQueue's `SELECT … FOR UPDATE SKIP LOCKED` and leader election).

#### Option B — Single durable lane (outbox only, no broadcast)
- **Pros**: one mental model; simpler ops.
- **Cons**: every event roundtrips through Postgres → 1-10ms baseline latency; live dashboards (Phase 11) become awkward; no clean answer for "react in-tx to validate before commit."
- **Invalidation rationale**: in-tx validation hooks (Surreal-style `ASSERT`, masking pre-write, anti-money-laundering checks) require sync delivery; outbox-only forces them into a different mechanism.

#### Option C — Actor-framework (actix / shaku) for hooks
- **Pros**: rich supervision tree, mailboxes, established pattern.
- **Cons**: actix has Send/!Send footguns, the broader ecosystem has moved away from it; doesn't solve CDC ingress; adds a heavy dep for a problem `tokio::broadcast` already solves.
- **Invalidation rationale**: actor framework is a *programming model* choice, not an *event* choice; you'd still need broadcast + outbox underneath. Pure cost.

#### Option D — External bus (NATS JetStream / Redis Streams)
- **Pros**: durable, replayable, multi-consumer cursors built-in.
- **Cons**: adds a service to run; cross-process IPC overhead; overkill on one machine; no closer to Rust hooks than Option A.
- **Invalidation rationale**: §1 principle 5 — specialist primitives only. NATS is the right answer at the second machine, the wrong answer on the first.

---

## 3. Architecture

```
   ┌──────────────────────────────────────────────────────────────────────────┐
   │                       producers (in-process)                             │
   │  tdw-runtime · tdw-agent · tdw-service · tdw-worker · tdw-mcp · tdw-cli  │
   └────────────────┬─────────────────────────────────┬───────────────────────┘
                    │                                 │
                    │ emit(typed event +              │ db write via
                    │       actor + trace ctx)        │ tower Service<DbCmd>
                    │                                 │   wrapped by HookLayer
                    ▼                                 ▼
   ┌─────────────────────────────────────┐   ┌──────────────────────────────┐
   │       tdw-bus (in-proc broadcast)   │   │      Postgres TX             │
   │   tokio::sync::broadcast<EE<P>>     │◀──┤  business row + outbox row   │
   │   per event-kind channel            │   │  (atomic, same commit)       │
   │   + async-broadcast for "no-loss"   │   └──────────────────────────────┘
   │   + watch<LatestState>              │                  │
   └────────┬────────────────────────────┘                  │ COMMIT
            │                                               ▼
            │ sync hooks                          ┌────────────────────────┐
            │ (in-tx, can veto)                   │  outbox table          │
            ▼                                     │  (FIFO, SKIP LOCKED)   │
   ┌───────────────────────────┐                  └─────────┬──────────────┘
   │   sync hook handlers      │                            │
   │   - validation            │                            │ RiverQueue
   │   - masking / RLS         │                            │ outbox_publisher
   │   - derived columns       │                            ▼
   │   - capability checks     │                  ┌─────────────────────────┐
   └───────────────────────────┘                  │  async hook handlers    │
                                                  │  - ClickHouse insert    │
                                                  │  - Qdrant embed         │
                                                  │  - Meili index          │
                                                  │  - S3 archive           │
                                                  │  - webhook notify       │
                                                  │  - live-WS broadcast    │
                                                  └────────┬────────────────┘
                                                           │
                                                           ▼
                                                  ┌─────────────────────────┐
                                                  │  event_archive          │
                                                  │  (append-only, partit.) │
                                                  │  + replay CLI           │
                                                  └─────────────────────────┘

   ┌─────────────────────────────── CDC INGRESS ─────────────────────────────┐
   │  Postgres WAL ──[pgoutput]──▶ pg_replicate / supabase-etl ──▶ tdw-bus   │
   │      OR (low-volume control plane)                                      │
   │  PL/pgSQL trigger ──[pg_notify]──▶ sqlx::PgListener ──▶ tdw-bus         │
   │      AND (ClickHouse changes)                                           │
   │  CH MaterializedView ──[remote/URL]──▶ tdw-bus  (insert-only)           │
   │  application emits events BEFORE CH write → CH treated as sink-only     │
   └─────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Event envelope + Actor context (concrete Rust)

```rust
// crates/tdw-event/src/envelope.rs

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Validate)]
pub struct EventEnvelope<P: EventPayload> {
    /// UUIDv7 — time-sortable; doubles as idempotency key.
    pub id: Uuid,
    /// CloudEvents-compatible spec version. Currently "1.0".
    pub spec_version: &'static str,
    /// Dotted, stable kind: "trade.filled", "instrument.updated".
    pub kind: &'static str,
    /// Emitter crate: "finx.execution", "finx.market_data".
    pub source: &'static str,
    /// Event time (not ingest time).
    pub occurred_at: DateTime<Utc>,
    /// Who/what triggered this event.
    pub actor: Actor,
    /// Where the event chain originated.
    pub origin: Origin,
    /// W3C trace context for cross-hop correlation.
    pub trace_context: TraceContext,
    /// Overflow business-context KVs (tenant, strategy_id, …).
    pub baggage: BTreeMap<String, String>,
    /// The event that caused this one (None for root events).
    pub causation_id: Option<Uuid>,
    /// Sticks to a logical workflow across many events.
    pub correlation_id: Uuid,
    /// Depth in the causation chain. Hard-capped at MAXDEPTH = 8.
    pub depth: u8,
    /// Typed payload.
    pub payload: P,
    /// Schema version (semver of the payload type).
    pub payload_schema_version: SemVer,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    User    { id: Uuid, email: String, role: String },
    Agent   { id: String, model: String, session_id: Uuid, parent: Option<Box<Actor>> },
    System  { component: &'static str },          // schedulers, migrations, boot
    Worker  { job_id: Uuid, queue: String, attempt: u32 },
    External{ source: String, request_id: String }, // exchange webhook, broker callback
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub enum Origin {
    /// Triggered by an external request (HTTP, MCP, CLI, agent tool-call entering the gateway).
    Client,
    /// Triggered internally (scheduler tick, replication apply, hook side-effect).
    Server,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TraceContext {
    pub traceparent: String,            // W3C "00-<trace-id>-<span-id>-<flags>"
    pub tracestate:  Option<String>,    // vendor opaque
}

pub trait EventPayload: Clone + Send + Sync + 'static
    + Serialize + DeserializeOwned + JsonSchema + Validate
{
    const KIND: &'static str;
    const SOURCE: &'static str;
    const SCHEMA_VERSION: SemVer;
}
```

Actor context also lives in a `tokio::task_local!`:

```rust
tokio::task_local! {
    pub static CURRENT_ACTOR: Actor;
    pub static CURRENT_TRACE: TraceContext;
    pub static CURRENT_ORIGIN: Origin;
    pub static CURRENT_DEPTH: AtomicU8;
    pub static FIRED_EVENTS: Mutex<HashSet<(StaticStr, String)>>; // (kind, pk) within tx
}
```

Auto-injected at every entry point (axum/tonic middleware, MCP server, RiverQueue worker, CLI dispatch). Auto-propagated by an `OmcSpawn` helper that wraps `tokio::spawn` to inherit task-locals.

---

## 5. Hook registration API

WordPress-style explicit priority + side + transaction mode + idempotency. Discovery via `linkme::distributed_slice` (compile-time, no init order issues).

```rust
// example hook from a downstream crate

use tdw_hooks::{hook, HookResult, Side, TxMode};

#[hook(
    on   = TradeFilled,
    kind = HookKind::Action,         // Action | Filter | Observer
    side = Side::Server,             // Client | Server | Both
    tx   = TxMode::PostCommit,       // Required | PostCommit | Detached
    priority = 100,                  // lower = earlier
    idempotent = true,
    max_depth = 3,
    enabled_by_default = true,
)]
async fn route_fill_to_compliance(
    ctx: &HookContext,
    ev: &EventEnvelope<TradeFilled>,
) -> HookResult {
    // Implementation. ctx.actor / ctx.trace / ctx.origin available.
    compliance::route(ev).await
}
```

### Hook semantics

| `kind` | Behavior | Return contract |
|--------|----------|-----------------|
| `Action`   | Fire-and-act side effect. Multiple handlers run; one failing does not affect others. | `Result<(), Error>` — error logged + DLQ'd |
| `Filter`   | Transform the payload. Handlers chain in priority order; output of N is input of N+1. | `Result<P, Error>` — error aborts the chain |
| `Observer` | Read-only audit/log. Cannot mutate payload, cannot fail the chain. | `Result<(), Error>` — error logged only |

### Transaction modes

| `tx` | When it runs | Can veto? |
|------|--------------|-----------|
| `Required`  | In the same Postgres transaction as the originating write | Yes — returning `Err` aborts the tx |
| `PostCommit`| After commit, via outbox → RiverQueue | No |
| `Detached`  | Immediately on the broadcast lane, no durability guarantee | No |

### Side flags

| `side` | Runs when | Enforcement |
|--------|-----------|-------------|
| `Client` | `Origin == Client` (external request) | Hook skipped on internal `Server` events |
| `Server` | `Origin == Server` | Skipped on `Client` events |
| `Both`   | Always | — |

The combination is checked at *dispatch time*; mismatch is a no-op (logged at trace level), never a crash. Misconfiguration (e.g. `Required + Detached`) is a compile-time error via type-state.

### Runtime enable/disable

A Postgres table `system.hook_state(hook_id text primary key, enabled bool, updated_at timestamptz)` overrides the compile-time default. `tdw-hooks` watches this table via the same CDC ingress (eats its own dog food) and toggles a `tokio::watch<HashSet<HookId>>`. CLI: `tdw-cli hooks enable|disable|list|describe <hook_id>`.

---

## 6. DB ingress paths

### Path A — High-volume (logical replication)

For every Postgres table where DB events drive Rust hooks at any non-trivial rate:

1. Create `PUBLICATION finx_finance FOR TABLE …` listing those tables.
2. Create a named replication slot (`finx_finance_main`) — one per consumer process.
3. `tdw-cdc` (built on `supabase/etl` / `pg_replicate`) tails the slot, decodes pgoutput, builds `EventEnvelope<TableRowChanged>`, and publishes to `tdw-bus`.
4. **`REPLICA IDENTITY FULL`** is set only on tables whose hooks need the old row (audit, soft-delete restore). Others stay at default to limit WAL volume.
5. Alert at 5 GB accumulated WAL on any slot (R44).

### Path B — Low-volume / control plane (LISTEN/NOTIFY)

For schema changes, config table updates, manual operator triggers:

1. PL/pgSQL trigger calls `pg_notify('finx.<channel>', json_build_object('event_id', uuid, 'table', 'foo', 'pk', new.id, 'op', 'INSERT', 'actor_jwt', current_setting('jwt.claims', true)))`.
2. Payload is **only the key** (under 8KB cap). Full row fetched on demand by the hook.
3. `tdw-cdc::listener` task consumes via `sqlx::PgListener` (auto-reconnect), publishes to `tdw-bus`.

### Path C — ClickHouse changes

CH is **sink-only**. Application code emits the event *before* (or atomically with) writing to CH. Don't tail CH for changes — confirmed in research §G. Internal CH-to-CH transforms use `MaterializedView` but those are warehouse-internal, not business hooks.

### Path D — Outbox (Rust → DB)

Every Rust crate writes via `tower::Service<DbCommand>` wrapped by `HookLayer<DbCommand>`:

1. Open Postgres tx.
2. Apply pre-write `Required` sync hooks (validation, masking, RLS check). If any returns `Err`, ROLLBACK.
3. Write business row.
4. Write outbox row in the same tx (`outbox(id uuid, kind text, payload jsonb, envelope jsonb, available_at, locked_until, attempts)`).
5. COMMIT.
6. RiverQueue's `outbox_publisher` worker dequeues with `FOR UPDATE SKIP LOCKED LIMIT 100` and dispatches to `PostCommit` async hooks.
7. Each event is also appended to `event_archive` (partitioned by month) for replay.

---

## 7. New crate set

```diff
crates/
+ tdw-event/                             ← EventEnvelope, Actor, Origin, TraceContext, EventPayload trait
+ tdw-bus/                               ← per-kind tokio::sync::broadcast; async-broadcast for no-loss; watch for latest-state
+ tdw-hooks/                             ← #[hook] proc-macro, linkme registry, HookContext, runtime enable table
+ tdw-outbox/                            ← outbox table schema, RiverQueue InsertTx integration, publisher worker
+ tdw-cdc/                               ← logical-replication consumer (pg_replicate) + PgListener fallback + CH MV bridge
+ tdw-replay/                            ← event_archive table, replay CLI, partition management
+ tdw-actor/                             ← Actor enum, task_local! plumbing, cap-std capability tokens, OmcSpawn helper
```

Sub-modules / extensions of existing crates:
- `tdw-service::middleware::actor_injection` — axum + tonic middleware that materialises `Actor` from JWT and scopes it into `CURRENT_ACTOR`.
- `tdw-mcp::actor` — MCP tool dispatcher injects `Actor::Agent { agent_id, model, session_id }` per call.
- `tdw-worker::actor` — RiverQueue worker entry restores `Actor::Worker { job_id, queue, attempt }` from job args.
- `tdw-storage-postgres::hook_layer` — `tower::Service<DbCommand>` wrapper for direction-2 writes.

Updated workspace total: 43 (Layer C) + 7 new (this layer) − **3 deleted** (`tdw-stream`, `tdw-live`, `tdw-notify` collapsed into spine adapters) = **47 crates**. Net +4 vs Layer C, but conceptually cleaner.

### Crates that get **deleted or absorbed**

| Crate (was planned in) | What happens |
|------------------------|--------------|
| `tdw-stream` (Phase 10, Layer C) | **Absorbed.** "CREATE STREAM" becomes a `Filter`-kind hook subscription with a durable cursor on `event_archive`. The crate becomes a single module `tdw-replay::stream`. |
| `tdw-live` (Phase 10, Layer C) | **Absorbed.** "LIVE SELECT" becomes a WebSocket adapter that subscribes to `tdw-bus` channels with WHERE-filter pushdown. Becomes a module `tdw-service::live`. |
| `tdw-notify` sub-module (Phase 9, Layer C) | **Absorbed.** Webhook + queue notifications become a built-in async hook handler in `tdw-hooks::sinks::webhook`. |

---

## 8. Refactoring impact on existing phases

The Layer C plan ([`2026-05-21-databend-surrealdb-feature-parity.md`](./2026-05-21-databend-surrealdb-feature-parity.md)) gets renumbered + simplified:

| Was | Renamed to | Net change |
|-----|------------|------------|
| Phase 9 — Snapshots / time travel / tags / notifications | **Phase 10** | Smaller — `tdw-notify` removed (absorbed by spine in Phase 9) |
| Phase 10 — Streams / CDC / live queries | **Phase 11** | **~50% smaller** — `tdw-stream` and `tdw-live` become spine adapters. Implementation collapses to: (a) durable consumer cursor on `event_archive`, (b) WebSocket bridge to `tdw-bus`. CDC plumbing already exists from Phase 9 (this plan). |
| Phase 11 — Graph + spatial | **Phase 12** | Unchanged. |
| Phase 12 — Stages / open table formats / pipes | **Phase 13** | Pipes refactored — auto-ingest is a `Server`-side `Action` hook on `S3.ObjectCreated` events. |
| Phase 13 — UDFs / auth / DEFINE / masking | **Phase 14** | DEFINE EVENT becomes a thin declarative wrapper that compiles to spine hook registrations. UDFs gain access to event emission via a `tdw-udf::emit` capability. Masking becomes a `Filter`-kind sync hook. |

**Phase 9 (new) = this plan.** Estimated 10 days.

---

## 9. Implementation Phases

### Phase 9 (new) — Hook & Event Spine — days 50–59

9.1. `tdw-event` core: `EventEnvelope<P>`, `Actor` enum, `Origin`, `TraceContext`, `EventPayload` trait, `SemVer`. JSON Schema export via `schemars`. 1.5d.

9.2. `tdw-actor`: `tokio::task_local!` plumbing, `Actor::current()` helper, `OmcSpawn` for inherited task-locals, `cap-std`-style capability tokens with phantom types. 1d.

9.3. `tdw-bus` core: per-kind `tokio::sync::broadcast` registry, `async-broadcast` for `no_loss = true` channels, `watch` for "latest state" channels (live tick per symbol, live config). Configurable channel capacities. Lag-counter metrics per subscriber. 1.5d.

9.4. `tdw-hooks` proc-macro + registry: `#[hook]` attribute, `linkme::distributed_slice` collection at startup, `HookContext`, sync/async dispatch with `priority` stable sort, Action/Filter/Observer kinds, recursive-depth guard, `(kind, pk)` re-entry set in task-local. 2d.

9.5. `tdw-outbox`: outbox table migration (`migrations/postgres/20260521_0010_outbox.sql`), `OutboxWriter` trait implemented by `tdw-storage-postgres`, RiverQueue `InsertTx` integration so the outbox row and the business row commit atomically, dedicated publisher worker with `FOR UPDATE SKIP LOCKED LIMIT 100`, exponential backoff retry with DLQ. 1.5d.

9.6. `tdw-cdc`: ingress paths. Path A: `supabase/etl` / `pg_replicate` consumer for logical replication (one slot per logical consumer, named per-deployment). Path B: `sqlx::PgListener` with auto-reconnect for control-plane NOTIFY. Path C: CH `MaterializedView` → `URL()` sink that posts to a local HTTP endpoint which republishes to `tdw-bus`. Each path produces typed `EventEnvelope<TableRowChanged>` and feeds the bus. 2d.

9.7. `tdw-replay`: `event_archive` table (PG, partitioned monthly), every committed event appended post-COMMIT by the outbox publisher, `tdw-cli replay --kind … --from … --to … --target …` CLI. 1d.

9.8. Wiring + middleware: `tdw-service::middleware::actor_injection` (axum + tonic), MCP actor injection, RiverQueue worker actor restoration, `OmcSpawn` helper. 0.5d.

9.9. Documentation: `docs/event-spine.md`, `docs/hook-authoring.md`, `docs/cdc-paths.md`, ADR-0021 (two-lane choice), ADR-0022 (envelope shape), ADR-0023 (recursive guard semantics), ADR-0024 (Origin tagging at entry points). 1d.

**Exit criteria**: A9.1–A9.16 satisfied (see §10).

### Phase 11 (renumbered from old Phase 10) — Streams + Live Queries adapters — days 67–70 (was 8 days, now ~4)

11.1. `tdw-replay::stream` — Databend-style `CREATE STREAM` as a durable cursor over `event_archive` keyed by `(table, replication_slot, last_seen_event_id)`. `SELECT * FROM STREAM(name)` queries the archive with the cursor. `APPEND_ONLY` filters by `op = 'INSERT'`. Stream-to-stream cloning copies the cursor.

11.2. `tdw-service::live` — WebSocket endpoint. On connect, parse `LIVE SELECT * FROM foo WHERE …` into a `tdw-bus` subscription with predicate. Permissions-aware via the actor JWT establishing PG RLS context server-side. DIFF mode via `json-patch` from `EventEnvelope.before`/`after`. `KILL <uuid>` closes the subscription. Auto-resume after disconnect via `event_archive` replay from `last_seen_event_id`.

11.3. `SHOW STREAMS` / `DESC STREAM` wraps a view on `system.stream_cursor`.

11.4. SurrealDB `CHANGEFEED` equivalent — table flag `changefeed = '3d'` auto-creates a hidden stream cursor with N-day retention.

11.5. Documentation: `docs/streams.md`, `docs/live-queries.md`.

**Exit criteria**: original A10.1–A10.12 satisfied (semantics unchanged; implementation now leans on Phase 9).

### Phase 14 (renumbered from old Phase 13) — slight tightening

DEFINE EVENT half of `tdw-define` now generates `#[hook]` registrations directly. UDFs gain a `tdw-udf::emit` capability. Masking implemented as a `Filter` sync hook. No new acceptance criteria; effort reduced by ~2 days.

---

## 10. Acceptance Criteria

### Phase 9 — Spine

A9.1. `EventEnvelope<P>` derives `Serialize + Deserialize + JsonSchema + Validate`; JSON Schema for every event kind exported to `schemas/events/`. Verified by `cargo xtask events emit-schemas` producing zero diff against committed schemas.
A9.2. `Actor` has all five variants (User/Agent/System/Worker/External); each round-trips through JSON byte-stable.
A9.3. `Origin::Client` is set by `tdw-service::middleware::actor_injection` on every axum request; `Origin::Server` is set by `OmcSpawn` for internal tasks. Verified by integration test that asserts origin tag on 10 representative emission sites.
A9.4. `tokio::task_local!` `CURRENT_ACTOR` is auto-propagated across `OmcSpawn::spawn` and across RiverQueue worker entry; verified by a "task hop" test that emits from a spawned child task and asserts the actor matches the parent.
A9.5. `tdw-bus`: `broadcast::Sender<EventEnvelope<P>>::send(...)` delivers to N subscribers in stable priority order; slow subscriber that lags >channel-capacity messages receives `Lagged(n)` and the publisher does not block.
A9.6. `async-broadcast` channel marked `no_loss = true` correctly blocks a publisher when the slowest subscriber is full; verified by 10k-message stress test with one slow subscriber.
A9.7. `#[hook]` macro registers handlers at compile time via `linkme::distributed_slice`; `cargo build` of a downstream crate that adds a hook makes it discoverable at runtime *without* editing any registry file.
A9.8. Hook ordering: 5 handlers on the same event with `priority = 100, 50, 10, 50, 200` run in `(10, 50, 50, 100, 200)` order; tie-broken by `hook_id`. Verified by deterministic-ordering test.
A9.9. `Action` hook that returns `Err` does NOT prevent other Action handlers on the same event from running; the error is logged + DLQ'd. `Filter` hook that returns `Err` aborts the chain. Verified by separate tests.
A9.10. Sync hook (`TxMode::Required`) returning `Err` rolls back the originating Postgres transaction; verified by a write test where the validator hook rejects and the row is not committed.
A9.11. Async hook (`TxMode::PostCommit`) runs after the originating commit; runtime crash between commit and async dispatch is recovered by RiverQueue on restart; verified by `kill -9` mid-flight test using `testcontainers`.
A9.12. Recursive guard: an event whose hook emits another event of the same `(kind, pk)` within the same `correlation_id` is rejected with `RecursionError`. Verified by a deliberate self-cycle.
A9.13. `MAXDEPTH = 8`: an event chain of 9 hops is rejected at hop 9 with `DepthExceeded`. Verified.
A9.14. Outbox atomicity: a Postgres tx that writes business row + outbox row, then `kill -9` before COMMIT, leaves neither committed; verified by integration test.
A9.15. CDC Path A: a row inserted into a published table appears as `EventEnvelope<TableRowChanged>` on the bus within 50 ms p95; verified against `testcontainers` Postgres with logical replication.
A9.16. CDC Path B: a `pg_notify('finx.config', …)` is delivered to a `PgListener` subscriber within 200 ms p95.
A9.17. Replay: `tdw-cli replay --kind trade.filled --from 2026-01-01 --to 2026-01-02 --target dryrun` enumerates the matching events in `event_archive`, prints count, never re-publishes by default.
A9.18. Runtime hook enable/disable: setting `system.hook_state.enabled = false` for a hook causes subsequent emissions to skip it within 500 ms; the change itself is delivered via the spine (eat-own-dogfood).

### Phase 11 — Refactored Streams + Live

(unchanged from original A10.1–A10.12; semantics same, implementation now sits on the spine.)

---

## 11. Risks & Mitigations

| #    | Risk | Likelihood | Impact | Mitigation |
|------|------|-----------|--------|------------|
| R44  | Postgres logical replication slot abandoned → WAL grows until disk full | **High** | **High** | Alert at 5 GB `pg_wal_lsn_diff(pg_current_wal_lsn(), confirmed_flush_lsn)`; CLI `tdw-cli cdc slots prune` removes orphaned slots; runtime warns on every connect if any slot is >24h stale. |
| R45  | `tokio::broadcast` `Lagged(n)` silently dropping events on critical channels | High | High | Critical channels (audit, compliance) use `async-broadcast` with `no_loss = true`; subscriber lag counter exported as a Prometheus metric; alarm at lag > 0 for critical channels. |
| R46  | Recursive event cycle escapes the `MAXDEPTH` guard via causation chain manipulation | Low | High | Depth counter is set by the bus dispatcher, not by hook code; hook code cannot override depth on re-emit; immutability enforced by builder type. |
| R47  | Two-lane mental model leads to hooks accidentally running in the wrong lane | Medium | Medium | `TxMode` declared at registration; compile-time type-state prevents `TxMode::Required + Origin::External`; runtime asserts lane vs handler signature. |
| R48  | Outbox publisher becomes a SPOF (one process draining the queue) | Medium | Medium | RiverQueue's `SELECT FOR UPDATE SKIP LOCKED` permits N publishers safely; production deploy can run 2; in personal scope, one is fine; auto-restart on crash via systemd / supervisor. |
| R49  | Outbox table grows unbounded if `event_archive` partitioning lags | High | Medium | After successful dispatch, outbox rows are deleted (not retained); `event_archive` is the durable history. Monthly partition rotation handled by `xtask partition`. |
| R50  | Slow async hook stalls the publisher → backlog grows | High | High | Per-hook timeout (`tower::timeout::Timeout`); slow hooks moved to a dedicated RiverQueue queue with its own concurrency limit; backlog metrics + alarm. |
| R51  | Actor identity lost when code uses raw `tokio::spawn` instead of `OmcSpawn` | High | Medium | Clippy lint via `disallowed-methods` (`clippy.toml`) blocks raw `tokio::spawn` outside `tdw-actor::OmcSpawn`; CI gate. |
| R52  | `JsonSchema` derive on `EventEnvelope` drifts from runtime `serde` representation | Medium | Medium | Schema-drift test in CI compares the runtime-emitted JSON of every event kind against its committed JSON Schema. |
| R53  | `MAXDEPTH = 8` too small for legitimate workflows | Low | Medium | Per-kind override on registration (`max_depth = 16`); workflow engine (Phase 8) explicitly raises depth for multi-step flows. |
| R54  | CDC ingress falls behind under burst write rate, makes hooks fire seconds late | High | Medium | Bus has direct in-process emit too — hot path emits to broadcast *before* writing to PG, so live consumers don't wait on the replication delay. Logical replication serves durable consumers (event_archive, CH sink). Two-path design accepts this asymmetry. |
| R55  | ClickHouse "sink-only" rule violated, accidental CH-tail hook added | Medium | Medium | `tdw-cdc` exposes no CH-tail API; CH `MaterializedView` ingress is documented as for warehouse-internal transforms only; PR checklist + lint. |
| R56  | Outbox + CDC double-publish on restart (race between WAL replay + outbox publisher) | Medium | High | UUIDv7 `event_id` is the dedupe key; downstream sinks (ClickHouse via `ReplacingMergeTree`, Qdrant via upsert, S3 via key) are idempotent on `event_id`. CI test asserts idempotent semantics. |
| R57  | `linkme` compile-time registration fails on Windows-MSVC + `lto=fat` (same family as R6) | Low | High | Fallback runtime registration path: each crate exposes a `register_hooks(&mut Registry)` fn called from `main`. CI release-profile Windows job (parent plan A10) exercises both paths. |
| R58  | Hook execution order tested in isolation but breaks under concurrent emissions | Medium | Medium | Property-based test (`proptest`) with random emission interleavings asserts that *per-event-instance* hook ordering is stable; across events, ordering is FIFO by `id` (UUIDv7). |

---

## 12. Verification Steps

V48. `cargo test -p tdw-event` — envelope serde + JSON Schema round-trip + Actor variants. (A9.1, A9.2)
V49. `cargo test -p tdw-actor --features integration` — task-local propagation across `OmcSpawn` + RiverQueue worker entry. (A9.3, A9.4)
V50. `cargo test -p tdw-bus --features integration` — broadcast + watch + async-broadcast under load; lag counter; no_loss blocking. (A9.5, A9.6)
V51. `cargo test -p tdw-hooks` — `#[hook]` macro discovery, priority ordering, kind semantics, side flags. (A9.7–A9.9)
V52. `cargo test -p tdw-storage-postgres --features hooks` — sync hook veto rolls back; async hook fires post-commit. (A9.10, A9.11)
V53. `proptest` recursion suite — random hook chains assert depth/cycle guards trigger correctly. (A9.12, A9.13)
V54. `cargo test -p tdw-outbox --features integration` — outbox atomicity via `testcontainers` + simulated crash. (A9.14)
V55. `cargo test -p tdw-cdc --features integration` — Path A logical replication latency p95 < 50ms; Path B NOTIFY latency p95 < 200ms. (A9.15, A9.16)
V56. `cargo test -p tdw-replay` — replay CLI enumerates correctly without re-publishing. (A9.17)
V57. `cargo test -p tdw-hooks --test runtime_toggle` — hook enable/disable propagates via spine within 500ms. (A9.18)
V58. **Adversarial test**: malicious downstream crate attempts `tokio::spawn(async move { /* no actor */ })`. Clippy lint catches it; CI fails. (R51)
V59. **Schema drift gate**: `xtask events schema-check` runs in CI; any event whose runtime JSON differs from committed schema fails the build. (R52)
V60. **Idempotency suite**: every downstream sink (CH, Qdrant, Meili, S3) is fired twice with the same `event_id`; second emission is a no-op. (R56)
V61. **End-to-end actor traceability**: a single trade fill emitted by `Actor::User { id: alice }` is followed through 6 hops (sync hooks, outbox, CH insert, Qdrant embed, audit log) and the audit table records `Actor::User { id: alice }` as the root of all 6 events. (A9.4 + integration)

---

## 13. ADR — Architecture Decision Record

- **Decision**: Build a two-lane hook & event spine (in-process `tokio::sync::broadcast` for hot path + Postgres outbox via RiverQueue for durable path) with a CloudEvents-shaped envelope that carries `Actor`, `Origin`, `TraceContext`, and a `MAXDEPTH = 8` recursive-event guard. Three hook kinds (Action / Filter / Observer) × three transaction modes (Required / PostCommit / Detached) × three side flags (Client / Server / Both), declared via a `#[hook]` proc-macro with compile-time registration. Replace `tdw-stream`, `tdw-live`, and `tdw-notify` (planned in Layer C) with thin adapters over this spine.

- **Drivers**:
  1. Bidirectional DB ↔ Rust event flow with one mental model.
  2. Personal single-machine scope — no Kafka/NATS/Debezium ops.
  3. Existing Layer C primitives (streams, live, notify, DEFINE EVENT) are fragmented; spine consolidates them.

- **Alternatives considered**:
  - **B — Single durable lane (outbox-only)**: rejected — no answer for in-tx validation hooks; live dashboards bottleneck on PG.
  - **C — Actor framework (actix)**: rejected — programming model, not event model; doesn't solve CDC ingress.
  - **D — External bus (NATS / Redis Streams)**: rejected — adds a service for no benefit on one machine.

- **Why chosen**:
  - Two independent research passes converged on this design.
  - Reuses crates already in the stack (`tokio`, `sqlx`, `RiverQueue`, `tower`, `tracing`, `axum`/`tonic`) + exactly one new dep (`pg_replicate` / `supabase/etl`).
  - Sync + async hooks typed at registration, not inferred → no "maybe async" surprises.
  - `Actor` enum + `Origin` flag + W3C trace context give first-class actor identity end-to-end without inventing a parallel auth system.
  - `MAXDEPTH = 8` (copied from Surreal) + `(kind, pk)` re-entry set: cheap, structural recursion safety.
  - Net code simplification across Phases 11 and 14 (Layer C) — ~5 days saved that partially offset the 10-day spine build.

- **Consequences**:
  - +7 new crates (`tdw-event`, `tdw-bus`, `tdw-hooks`, `tdw-outbox`, `tdw-cdc`, `tdw-replay`, `tdw-actor`) and 3 deleted (`tdw-stream`, `tdw-live`, `tdw-notify` absorbed). Net +4 → workspace at ~47 crates.
  - Postgres becomes mandatory for any durable event path (the outbox + RiverQueue both live there). ClickHouse, Qdrant, Meilisearch, S3 are sinks only.
  - Logical replication slot management is a new ops concern (R44).
  - Phase numbering shifts: old 9→10, 10→11, 11→12, 12→13, 13→14. Spine is new Phase 9.
  - Total project timeline: previously ~97 days, now ~102 days (spine +10, refactored Phase 11 −5, slight tightening on 14).

- **Follow-ups**:
  - ADR-0021 — two-lane vs outbox-only design rationale (this plan)
  - ADR-0022 — envelope shape + actor enum
  - ADR-0023 — MAXDEPTH semantics + (kind, pk) re-entry set
  - ADR-0024 — Origin tagging policy at entry points
  - ADR-0025 — relationship to DEFINE EVENT (Phase 14)
  - O18 — should the spine support cross-process replay (NATS JetStream as a future drop-in)? (Default: no, but envelope is binding-compatible with CloudEvents HTTP binding if needed.)
  - O19 — should `Filter`-kind hooks be allowed to mutate `Actor` / `TraceContext`? (Default: no — only `payload` is mutable; envelope is immutable for filters.)
  - O20 — multi-publisher outbox for HA — when (if ever)?

---

## 14. Combined timeline (updated)

| Phase | Description | Days | Cumulative |
|-------|-------------|------|------------|
| 0.0   | Discovery (BOM re-derive, ADRs, license) | 0–2 | 2 |
| 0.1   | Workspace skeleton + CI matrix | 1 | 3 |
| 1     | Core abstractions | 2–5 | 8 |
| 2     | Storage engines | 6–10 | 13 |
| 3     | First providers | 11–13 | 16 |
| 4     | Hybrid retrieval + 3 embedders | 14–20 | 23 |
| 5     | Four consumer shells | 21–26 | 29 |
| 6     | Hardening & docs | 27–32 | 35 |
| 7     | Data engineering (dbt, SQL, ETL/ELT, DDL codegen) | 33–42 | 45 |
| 8     | Agent schemas (12 types, MCP tools, eval runner) | 43–49 | 52 |
| **9 (NEW)** | **Hook & event spine** | **50–59** | **62** |
| 10 (was 9)  | Snapshots / time travel / tags | 60–68 | 70 |
| 11 (was 10) | Streams + live queries (now adapters) | 69–72 | 73 |
| 12 (was 11) | Graph + spatial | 73–80 | 80 |
| 13 (was 12) | Stages + table formats + pipes | 81–90 | 90 |
| 14 (was 13) | UDFs + auth + DEFINE + masking | 91–100 | 100 |

Total **~100 days serial / ~70 days with parallelization** (Phase 7 can overlap with Phase 8; Phases 12+13 can run alongside 14 after Phase 11). Spine is on the critical path — cannot parallelize.

---

## 15. Open Questions

- **O18** — Cross-process replay (NATS JetStream drop-in) at v0.2 or never?
- **O19** — `Filter` hooks mutating envelope fields beyond `payload` — allowed or not? (Default: not.)
- **O20** — Multi-publisher outbox HA — when, if ever?
- **O21** — Should ClickHouse hot-path inserts also fire spine events (currently only Postgres does)? Implies dual-write or a CH `MaterializedView` → HTTP → spine bridge.
- **O22** — `Action` hook failure DLQ retention — how long? (Default: 30 days, archive to S3 after.)
- **O23** — Hooks running as Python/JS/WASM UDFs (Phase 14) — same `#[hook]` registration or separate? (Likely separate registration, but emit through same spine.)

---

## 16. Changelog

**2026-05-21 — Layer E plan: hook & event spine**
- Two parallel research passes (Rust event-bus crate comparison + bidirectional event-system patterns) converged on a two-lane design.
- 7 new crates: `tdw-event`, `tdw-bus`, `tdw-hooks`, `tdw-outbox`, `tdw-cdc`, `tdw-replay`, `tdw-actor`.
- 3 Layer C crates collapsed into spine adapters: `tdw-stream` → `tdw-replay::stream`, `tdw-live` → `tdw-service::live`, `tdw-notify` → `tdw-hooks::sinks::webhook`.
- Phase numbering shift: old 9–13 become 10–14; new Phase 9 = this spine.
- 18 acceptance criteria (A9.1–A9.18), 15 risks (R44–R58), 14 verification steps (V48–V61), 6 open questions (O18–O23).
- 5 new ADRs (0021–0025).
- Total timeline now ~100 days serial / ~70 days parallelized.
