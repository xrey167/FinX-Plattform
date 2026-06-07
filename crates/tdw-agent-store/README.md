# tdw-agent-store

In-memory persistence for agent entities plus the running memory-consolidation
loop.

## Purpose

Two pieces:

1. **[`AgentStore`]** — an in-memory store (BTree-backed) for agent cards,
   gotchas, workflow definitions, and recorded eval runs, with both unchecked
   (`upsert_*`/`record_eval_run`) and checked (`try_*`) mutation paths.
2. **[`MemoryStore`] + consolidation** — a `Memory`-entity store with optional
   JSON5 file backing, a deterministic apply step ([`consolidate_at`]) over the
   pure `tdw_agent::consolidation_plan` planner, and a periodic background
   scheduler ([`spawn_consolidation_scheduler`]).

Consolidation is deterministic by injecting `now` (an RFC 3339 string) into
[`consolidate_at`] / [`age_days`]; only the live scheduler reads the wall clock.

## Feature flags

None. Dependencies: `serde`, `serde_json`, `tdw-agent`, `chrono`, `tokio`,
`tokio-util`.

## Environment variables

None.

## Quickstart

```rust
use tdw_agent::sample_agent_card;
use tdw_agent_store::AgentStore;

let mut store = AgentStore::new();
let card = sample_agent_card();
store.upsert_agent(card.clone());
assert_eq!(store.agent("market-researcher"), Some(&card));
```

Memory consolidation (deterministic, in-memory):

```rust
use tdw_agent_store::{MemoryStore, consolidate_at};

let mut store = MemoryStore::new();
// ... upsert_at(memory, now) ...
let actions = consolidate_at(&mut store, "2026-05-10T00:00:00Z")?;
# Ok::<(), tdw_agent_store::MemoryStoreError>(())
```

`MemoryStore::load_dir(dir)` backs the store with `*.json5` files so a tier change
survives a restart; `MemoryStore::new()` is purely in-memory.

## Example

```text
cargo run --example tdw_agent_store_basic -p tdw-agent-store
```

`examples/basic.rs` runs an in-memory `AgentStore` round-trip (agent + workflow +
eval run) and shows the checked-path rejection — no filesystem, no scheduler.
