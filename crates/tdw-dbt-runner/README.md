# tdw-dbt-runner

Builds validated `dbt run` command specifications and parses `dbt`'s
`run_results.json` output. It is the safe boundary between the platform's job
model and the external `dbt` CLI.

## Purpose

The crate does two things, both offline and pure:

- **Command building.** `DbtCommand::build_run(project_dir, selector)` (and the
  `…_with_profiles` variant) produce a validated `DbtCommand { project_dir,
  profiles_dir, args }`. Inputs are checked for emptiness and control characters,
  and the `args` vector is assembled as `["run", "--select", selector]` — never via
  a shell, so there is no injection surface.
- **Result parsing.** `parse_run_results(json)` deserializes a dbt
  `run_results.json` body into `DbtRunResult`; `run_step_rows` flattens it into
  `(node_id, status, execution_time)` tuples for reporting.

The crate does **not** execute `dbt` — it only models the command and reads the
results. `#![forbid(unsafe_code)]`.

## Feature flags

None.

## Dependencies

- `serde`, `serde_json` — result (de)serialization.
- `thiserror` — `DbtCommandError` variants.

## Quickstart

```rust
use tdw_dbt_runner::{DbtCommand, parse_run_results, run_step_rows};

// Build a validated `dbt run --select <selector>` command.
let cmd = DbtCommand::build_run("dbt/finx_finance", "tag:layer:bronze")?;
assert_eq!(cmd.args, vec!["run".to_string(), "--select".to_string(), "tag:layer:bronze".to_string()]);

// Parse a run_results.json body.
let result = parse_run_results(
    r#"{"results":[{"unique_id":"model.proj.bronze_ohlcv","status":"success","execution_time":0.12}]}"#,
)?;
let rows = run_step_rows(&result);
assert_eq!(rows[0].1, "success");
# Ok::<(), Box<dyn std::error::Error>>(())
```

Run the worked example:

```text
cargo run -p tdw-dbt-runner --example basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — command contract and result model.
- `tdw-pipeline` — the job DAG whose stages these commands implement.
