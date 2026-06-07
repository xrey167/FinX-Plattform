# tdw-replay

Dry-run replay planning over change-data-capture (CDC) and protocol rollout
records: see *what would be replayed* without mutating anything.

`tdw-replay` turns recorded history into a replay plan. From `tdw-cdc`
`CdcRecord`s it produces a `ReplayPlan` (offsets + event ids, always
`dry_run = true`); from `tdw-rollout` `RolloutRecord`s it produces a
`ProtocolReplayPlan` (sequence numbers + protocol event-type names). Neither path
performs the replay — they describe it.

## What it provides

- `ReplayEngine::dry_run(&[CdcRecord]) -> ReplayPlan` — CDC replay plan.
- `ReplayEngine::from_rollout(&[RolloutRecord]) -> ProtocolReplayPlan` — protocol
  rollout replay plan.
- `ReplayPlan` (`{ dry_run, offsets, event_ids }`).
- `ProtocolReplayPlan` (`{ sequences, event_types }`).

## Feature flags

None. Depends on `serde`, `tdw-cdc`, `tdw-protocol`, and `tdw-rollout`.

## Quickstart

```rust
use tdw_replay::ReplayEngine;
use tdw_rollout::RolloutRecord;
// build RolloutRecords from tdw-protocol ReplayFrames...

// let plan = ReplayEngine::from_rollout(&records);
// assert_eq!(plan.sequences, vec![1]);
// assert_eq!(plan.event_types, vec!["completed".to_string()]);
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-replay --example tdw_replay_basic
```

## Plan contract

- `dry_run` collects each `CdcRecord`'s `offset` and `event_id` into a
  `ReplayPlan` with `dry_run = true` — it reads the records and reports what a
  replay would touch, never writing.
- `from_rollout` maps each `RolloutRecord`'s `frame.sequence` and the *name* of
  its `frame.event` (via an internal exhaustive match over `tdw-protocol`'s
  `EventMsg` variants — `started`, `progress`, `approval_requested`,
  `tool_call_requested`, `tool_call_completed`, `output_chunk`, `domain_event`,
  `completed`, `failed`, `cancelled`).

## Invariants

- `#![forbid(unsafe_code)]`.
- **Dry-run only.** Planning reads recorded history and produces a description;
  it performs no replay and no mutation.
- **Exhaustive event-type mapping.** The protocol `EventMsg` match is total, so a
  new variant forces a compile-time update rather than a silent mislabel.
- **Deterministic** — plans are pure projections of their input slices.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
