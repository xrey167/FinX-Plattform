# tdw-pipe

Incremental, offset-tracked ingestion pipes. A `PipeDefinition` binds an external
stage to a target table and remembers how far it has loaded, turning a one-shot
`tdw-stage` `COPY INTO` into a resumable, continuous loader.

## Purpose

Where `tdw-stage` describes a *single* load plan, a `PipeDefinition` is the
*standing* ingestion object:

- it owns a `StageLocation` and a `target_table`;
- `copy_plan(files)` produces a validated `tdw_stage::CopyIntoPlan` for a fresh
  batch of files;
- `advance(offset)` records progress with a monotonic high-water mark
  (`last_offset = max(last_offset, offset)`), so out-of-order acks never move the
  cursor backwards.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde` — `PipeDefinition` (de)serialization.
- `tdw-stage` — `StageLocation`, `CopyIntoPlan`, and the `StageResult` error type.

## Quickstart

```rust
use tdw_pipe::PipeDefinition;
use tdw_stage::StageLocation;

let mut pipe = PipeDefinition {
    name: "market-pipe".to_string(),
    stage: StageLocation {
        name: "market-stage".to_string(),
        uri: "s3://bucket/market".to_string(),
    },
    target_table: "raw.market_data_bar".to_string(),
    last_offset: 0,
};

let plan = pipe.copy_plan(vec!["ohlcv.parquet".to_string()]).expect("valid plan");
assert_eq!(plan.target_table, "raw.market_data_bar");

pipe.advance(42);
pipe.advance(7); // earlier offset is ignored
assert_eq!(pipe.last_offset, 42);
```

Run the worked example:

```text
cargo run -p tdw-pipe --example tdw-pipe-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — pipe/offset model and stage composition.
- `tdw-stage` — the load-plan primitive a pipe composes.
