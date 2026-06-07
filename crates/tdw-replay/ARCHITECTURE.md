# tdw-replay architecture

A single-module crate (`src/lib.rs`): dry-run replay planning over CDC and
protocol rollout history.

## Module map

| Item | Role |
|------|------|
| `ReplayPlan` | `{ dry_run, offsets, event_ids }` — a CDC replay plan. |
| `ProtocolReplayPlan` | `{ sequences, event_types }` — a protocol rollout replay plan. |
| `ReplayEngine` | The (stateless) planner: `dry_run`, `from_rollout`. |
| `event_type_name` (private, `const fn`) | Exhaustive `EventMsg` → `&'static str` name. |

## Contract

`tdw-replay` deliberately does the *planning* half of replay and nothing else: it
reads recorded records and emits a structured description of what a replay would
do. There is no execution path, so the crate cannot mutate state by construction.

### CDC path — `dry_run`

`ReplayEngine::dry_run(&[CdcRecord])` walks the records and collects their
`offset`s and `event_id`s into a `ReplayPlan` with `dry_run = true`. The
`dry_run` flag is hard-coded true: this entry point is the *describe, don't
apply* path.

### Rollout path — `from_rollout`

`ReplayEngine::from_rollout(&[RolloutRecord])` maps each record's
`frame.sequence` and the **name** of its `frame.event`. The name comes from
`event_type_name`, a `const fn` with an **exhaustive** match over every
`tdw-protocol::EventMsg` variant (`started`, `progress`, `approval_requested`,
`tool_call_requested`, `tool_call_completed`, `output_chunk`, `domain_event`,
`completed`, `failed`, `cancelled`). Because the match is exhaustive, adding a
new protocol event variant is a compile error here until it is handled —
preventing a silently mislabeled replay plan.

## Dependencies

The crate bridges three record sources:

- `tdw-cdc` — `CdcRecord` (offset/event-id/event-type/payload).
- `tdw-rollout` — `RolloutRecord` (recorded-at + a protocol `ReplayFrame`).
- `tdw-protocol` — `ReplayFrame`, `EventMsg`, `SessionId`, `OpId`.

It depends on their *types* only; it neither reads from a store nor writes to
one.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Dry-run only** — planning never mutates; the CDC plan is hard-flagged
  `dry_run`.
- **Exhaustive, total event-name mapping** — new `EventMsg` variants must be
  handled at compile time.
- **Deterministic, pure projections** of the input slices.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
