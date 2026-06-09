# tdw-pipeline — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `PipelineJob` | One job: `name`, `runner`, `args`, `depends_on` (all `&'static str`). |
| `PipelineValidationError` | All structural failures (see invariants). |
| `Result<T>` | `std::result::Result<T, PipelineValidationError>` alias. |
| `market_data_dbt_jobs` | Built-in bronze→silver→gold→test market-data DAG. |
| `validate_jobs` | Whole-DAG structural validation. |
| `can_enqueue` | Dependency-satisfaction check for one job. |

## Key types and traits

- `PipelineJob` derives `Clone, Debug, PartialEq, Eq`. Its fields are `&'static
  str` / `&'static [&'static str]`, so a pipeline is normally a `const`/`static`
  literal with no heap allocation.
- `PipelineValidationError` uses `thiserror`; several variants carry the offending
  `name` (and `dependency`) for actionable messages.

## DAG model / data flow

```
[PipelineJob, ...]  (e.g. market_data_dbt_jobs())
        │
        │ validate_jobs(&jobs)
        ▼
  pass 1: per-job field checks + duplicate-name detection (BTreeSet of names)
  pass 2: per-edge checks — self-dependency? dependency exists in name set?
        ▼
   Ok(())  |  Err(PipelineValidationError)

scheduling loop (caller):
   for each job: can_enqueue(job, &completed) ?  // all depends_on ∈ completed
        ▶ run, then add job.name to completed
```

`validate_jobs` proves the DAG is structurally sound up front; `can_enqueue` is
the per-tick readiness predicate a scheduler calls to decide what to dispatch
next. Cycle *prevention* here is by construction (dependencies must reference
already-named jobs and cannot be self-referential); for free-form graphs use
`tdw-graph::has_cycle`.

## Invariants

- `validate_jobs` rejects: empty pipeline (`EmptyPipeline`); empty `name`
  (`EmptyJobName`), `runner` (`EmptyRunner`), or `args` (`EmptyArgs`); duplicate
  job names (`DuplicateJob`); a job depending on itself (`SelfDependency`); a
  dependency that names no known job (`UnknownDependency`).
- Field checks run before edge checks, and duplicate detection happens in pass 1
  so edge validation can rely on a complete, de-duplicated name set.
- `can_enqueue` returns `true` only when **every** entry in `depends_on` is present
  in `completed_jobs`; a job with no dependencies is always enqueueable.
- Pure and deterministic: no I/O, no global state.
