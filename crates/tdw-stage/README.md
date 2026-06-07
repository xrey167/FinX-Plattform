# tdw-stage

External-stage definitions and validated `COPY INTO` load plans — the building
block for bulk-loading files from object storage into warehouse tables.

## Purpose

A `StageLocation` names an external location (e.g. an S3 prefix). A
`CopyIntoPlan` is a validated description of "copy these files from this stage
into this table", carrying a content checksum so the plan can be verified before
execution.

`CopyIntoPlan::new` validates every boundary (non-empty stage name/uri, non-empty
target table, at least one non-empty file path) and computes a checksum over the
file list; `validate()` re-checks the boundaries **and** that the stored checksum
still matches the file list, catching tampering or drift.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async. It performs no
actual copy — it produces and validates the *plan*.

## Feature flags

None.

## Dependencies

- `serde` — `StageLocation` / `CopyIntoPlan` (de)serialization.
- `thiserror` — `StagePlanError` variants.

## Quickstart

```rust
use tdw_stage::{CopyIntoPlan, StageLocation};

let plan = CopyIntoPlan::new(
    StageLocation {
        name: "market-stage".to_string(),
        uri: "s3://bucket/market".to_string(),
    },
    "raw.market_data_bar",
    vec!["ohlcv.parquet".to_string()],
)
.expect("valid copy plan");

assert_eq!(plan.target_table, "raw.market_data_bar");
assert!(plan.checksum > 0);
assert!(plan.validate().is_ok());
```

Run the worked example:

```text
cargo run -p tdw-stage --example basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — plan/checksum model and error contract.
- `tdw-pipe` — wraps a stage into an incremental, offset-tracked pipe.
