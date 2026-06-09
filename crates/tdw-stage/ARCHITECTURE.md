# tdw-stage — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `StageLocation` | External location (`name`, `uri`). |
| `CopyIntoPlan` | Validated load plan (`stage`, `target_table`, `files`, `checksum`). |
| `StagePlanError` | All validation failures (see below). |
| `Result<T>` | `std::result::Result<T, StagePlanError>` alias. |
| `CopyIntoPlan::new` | Build + checksum + validate. |
| `CopyIntoPlan::validate` | Re-check boundaries and checksum. |
| `calculate_checksum` | Internal: sum of file-path bytes. |

## Key types and traits

- `StageLocation` and `CopyIntoPlan` derive `Clone, Debug, PartialEq, Eq,
  Serialize, Deserialize`.
- `StagePlanError` derives `Debug, Error, PartialEq, Eq` with `thiserror` messages.
  Variants: `EmptyStageName`, `EmptyStageUri`, `EmptyTargetTable`, `EmptyFiles`,
  `EmptyFilePath`, `ChecksumMismatch { expected, actual }`.

## Data flow

```
StageLocation + target_table + files
        │
        ▼
CopyIntoPlan::new(stage, target_table, files)
        │  checksum = calculate_checksum(&files)   // sum of file-path bytes
        │  self.validate()?
        ▼
CopyIntoPlan { stage, target_table, files, checksum }
        │
        ▼ (later, after transport/round-trip)
plan.validate()  // re-checks all boundaries AND recomputes checksum vs stored
```

The checksum is a deliberately simple, deterministic fold (sum of UTF-8 bytes of
every file path). Its purpose is integrity/drift detection of the *file list*
across (de)serialization, not cryptographic security.

## Invariants

- A `CopyIntoPlan` returned by `new` is always valid at construction time.
- `validate()` fails with the **first** boundary error encountered, in order:
  stage name → stage uri → target table → empty file list → empty file path →
  checksum mismatch.
- `checksum` must equal `calculate_checksum(files)`; mutating `files` without
  recomputing (or mutating `checksum` directly) makes `validate()` return
  `ChecksumMismatch { expected, actual }`.
- The crate models the plan only; it never touches object storage or the
  warehouse. Execution is the caller's responsibility.
