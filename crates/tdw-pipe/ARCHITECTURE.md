# tdw-pipe — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `PipeDefinition` | Standing pipe: `name`, `stage`, `target_table`, `last_offset`. |
| `PipeDefinition::copy_plan` | Build a validated `CopyIntoPlan` for a file batch. |
| `PipeDefinition::advance` | Move the monotonic offset high-water mark forward. |

## Key types and traits

- `PipeDefinition` derives `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`,
  so a pipe (including its current `last_offset`) can be persisted and restored.
- It reuses `tdw_stage::CopyIntoPlan`, `StageLocation`, and the
  `Result as StageResult` alias — `copy_plan` simply forwards stage/table plus the
  caller's file list into `CopyIntoPlan::new`, inheriting its validation.

## Stage composition / data flow

```
PipeDefinition { stage, target_table, last_offset }
        │
        │ copy_plan(files)
        ▼
CopyIntoPlan::new(stage.clone(), target_table.clone(), files)  ─▶ StageResult<CopyIntoPlan>
        │ (validated + checksummed by tdw-stage)
        ▼
      execute (caller) ─▶ ack offsets
        │
        │ advance(offset)
        ▼
last_offset = max(last_offset, offset)   // monotonic high-water mark
```

A pipe is therefore a thin, stateful wrapper that (a) delegates load-plan
construction/validation to `tdw-stage` and (b) tracks ingestion progress so the
next batch can resume from the right place.

## Invariants

- `copy_plan` produces only valid plans — it propagates any `StagePlanError` from
  `CopyIntoPlan::new` (e.g. empty file list) rather than building an invalid plan.
- `advance` is **monotonic**: `last_offset` never decreases. Replaying an older
  offset (`advance(7)` after `advance(42)`) is a no-op on the cursor.
- The pipe holds no I/O; it composes the pure `tdw-stage` primitive and owns only
  the progress cursor. Actual file transfer and offset acknowledgement are the
  caller's responsibility.
