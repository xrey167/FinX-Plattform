# tdw-agent-store — Architecture

## Module map

| File | Role |
| --- | --- |
| `src/lib.rs` | [`AgentStore`] (agents, gotchas, workflows, eval runs) + `StoredEvalRun` + `StoreError`. |
| `src/memory.rs` | [`MemoryStore`], [`consolidate_at`], [`age_days`], [`spawn_consolidation_scheduler`], `MemoryStoreError`. |

## `AgentStore`

Four `BTreeMap`s keyed by id: `agents`, `gotchas`, `workflows`, `eval_runs`.
Each mutation has two tiers:

| Unchecked | Checked |
| --- | --- |
| `upsert_agent` | `try_upsert_agent` → `validate_agent_card_contract` |
| `upsert_workflow` | `try_upsert_workflow` → `validate_workflow_contract` |
| `record_eval_run` | `try_record_eval_run` → run-id/agent-id/dataset-id non-empty, finite metric values |

`StoredEvalRun` carries an additive `updated_skills` field (skipped when empty)
so pre-feedback runs serialize identically.

## `MemoryStore` + consolidation

The pure planner lives in `tdw_agent::consolidate`; this crate owns the **state
and the running process**.

- An `Entry` is `{ memory, path: Option<PathBuf> }`. `MemoryStore::new()` is
  in-memory only; `MemoryStore::load_dir(dir)` loads every `*.json5` and remembers
  each file path so later mutations persist back.
- `upsert_at(memory, now)` stamps `last_consolidated = now` when absent (so a
  fresh memory ages from insertion), then persists if backed by a dir.
- `consolidate_at(store, now)`:
  1. builds `(memory, age_days)` pairs (`age_days` is RFC-3339 arithmetic that
     saturates at 0 and treats unparseable/None as 0 — never spuriously
     promotes/expires),
  2. calls the pure `consolidation_plan`,
  3. applies each action — `Promote` rewrites tier + `last_consolidated` and
     persists; `Expire` removes the entry (and deletes its file).
  A persistence failure aborts remaining actions and surfaces the error, so a
  tier change is never silently lost.

### Determinism boundary

`now` is a parameter everywhere except `spawn_consolidation_scheduler`, which is
the **only** place `chrono::Utc::now()` is read. The scheduler ticks, locks the
store, and consolidates; a tick error is logged and the loop continues; the apply
step is wrapped in `catch_unwind` so one panicking tick can never *silently* end
the task (tokio's `Mutex` does not poison). Shutdown is cooperative via a
`CancellationToken`. This mirrors `tdw_app_server::spawn_inmemory_relay`.

## Offline test design

`AgentStore` tests are pure in-memory round-trips. `MemoryStore` tests use unique
temp directories for the JSON5 round-trip / expire-deletes-file cases and an
injected `now` for the deterministic apply step; the scheduler test drives the
real clock only to prove start/stop. No network anywhere.
