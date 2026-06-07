# tdw-pipeline

A small job-DAG model with validation and ready-to-run scheduling checks. It
describes a set of jobs, their runners/args and `depends_on` edges, and answers
"is this DAG well-formed?" and "can this job run yet?".

## Purpose

`tdw-pipeline` models batch pipelines (e.g. the bronze → silver → gold → test dbt
flow) as a list of `PipelineJob`s. It provides:

- `validate_jobs` — structural checks over the whole DAG (no empties, no
  duplicates, no self-dependency, no unknown dependency);
- `can_enqueue` — whether a job's dependencies are all satisfied given the set of
  already-completed jobs;
- `market_data_dbt_jobs` — a built-in 4-stage market-data dbt pipeline used as the
  canonical example and as a fixture.

Jobs use `&'static str` fields, so a pipeline is typically declared as a static
constant with zero allocation.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `thiserror` — `PipelineValidationError` variants.

## Quickstart

```rust
use tdw_pipeline::{can_enqueue, market_data_dbt_jobs, validate_jobs};

let jobs = market_data_dbt_jobs();
validate_jobs(&jobs).expect("pipeline is well-formed");

let silver = jobs.iter().find(|j| j.name == "dbt_silver_market_data").unwrap();

// Silver cannot run until bronze has completed.
assert!(!can_enqueue(silver, &[]));
assert!(can_enqueue(silver, &["dbt_bronze_market_data"]));
```

Run the worked example:

```text
cargo run -p tdw-pipeline --example tdw-pipeline-basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — DAG validation and enqueue model.
- `tdw-graph` — lower-level directed-graph traversal/cycle detection.
- `tdw-dbt-runner` — builds the actual dbt commands these jobs run.
