# tdw-dbt-runner — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `DbtCommand` | Validated command spec: `project_dir`, `profiles_dir`, `args`. |
| `DbtRunResult` / `DbtNodeResult` | Parsed `run_results.json` shape. |
| `DbtCommandError` | `EmptyProjectDir`, `EmptyProfilesDir`, `EmptySelector`, `SelectorControlCharacters`. |
| `Result<T>` | `std::result::Result<T, DbtCommandError>` alias. |
| `DbtCommand::build_run` | Build with the default profiles dir. |
| `DbtCommand::build_run_with_profiles` | Build with an explicit profiles dir. |
| `parse_run_results` | Deserialize a `run_results.json` body. |
| `run_step_rows` | Flatten results into `(node_id, status, execution_time)`. |
| `validate_component` (private) | Non-empty check helper. |

## Key types and traits

- `DbtCommand` derives `Clone, Debug, PartialEq, Eq`.
- `DbtRunResult` / `DbtNodeResult` derive `Clone, Debug, PartialEq, Serialize,
  Deserialize`. `DbtNodeResult.node_id` is `#[serde(rename = "unique_id")]` to map
  dbt's field name.
- `DbtCommandError` uses `thiserror`.

## Command + result contract

```
build_run(project_dir, selector)
    └─ build_run_with_profiles(project_dir, "dbt/finx_finance", selector)
           validate_component(project_dir)   else EmptyProjectDir
           validate_component(profiles_dir)   else EmptyProfilesDir
           validate_component(selector)       else EmptySelector
           selector has no control chars?     else SelectorControlCharacters
           ▶ DbtCommand { project_dir, profiles_dir, args: ["run", "--select", selector] }

parse_run_results(json) ──serde_json──▶ DbtRunResult { results: [DbtNodeResult, ...] }
run_step_rows(&result)  ──▶ [(node_id, status, execution_time), ...]
```

The command is always assembled as a discrete argv vector
(`["run", "--select", <selector>]`) — there is **no shell string**, so a selector
can never inject extra commands. Control characters are additionally rejected so a
selector cannot smuggle newlines into a log or downstream consumer.

## Invariants

- `project_dir`, `profiles_dir`, and `selector` must all be non-empty (after trim);
  the first empty one determines the error.
- The selector must contain no control characters (`SelectorControlCharacters`).
- The default profiles directory is `dbt/finx_finance` when `build_run` is used.
- Emitted `args` are exactly `["run", "--select", <selector>]`, in that order.
- `DbtNodeResult` round-trips dbt's `unique_id` field via the rename; parsing is a
  straight `serde_json` deserialize and surfaces malformed JSON as a
  `serde_json::Error`.
- The crate never spawns a process; building and parsing are pure and offline.
