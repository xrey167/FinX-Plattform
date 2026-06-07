# tdw-define — Architecture

## Module map

Single `lib.rs`:

| Item | Role |
| --- | --- |
| `DefineEvent` | The declaration record (`event_name`, `on_table`, `hook_name`, `transaction_mode`). |
| `DefineError` | `InvalidEventName`, `InvalidTableName`, `InvalidHookName`. |
| `DefineEvent::compile_hook` | Infallible compile into `tdw_hooks::HookSpec`. |
| `DefineEvent::try_compile_hook` | `validate()` then `compile_hook()`. |
| `DefineEvent::idempotency_key` | Deterministic dedup key. |
| `is_action_name` / `is_table_name` / `is_table_part` | Internal grammar checks. |

## Key types and traits

- **`DefineEvent`** derives `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`.
- **`DefineError`** is a `Copy` enum, also `serde`-serializable.
- The compile target is `tdw_hooks::HookSpec`, built via
  `HookSpec::new(hook_name, 100, transaction_mode)` — order `100` is the default
  priority assigned by the `DEFINE` front end.

## Data flow

```
DefineEvent ──validate()──▶ Result<(), DefineError>
     │  (event_name / on_table / hook_name grammar)
     ▼
compile_hook() ──▶ HookSpec { name, order=100, event=Custom(name), handler=Command{name}, ... }
     │
     ▼
idempotency_key() = "{on_table}:{event_name}:{hook_name}"
```

`try_compile_hook` is the safe entry point: it validates first and only compiles
on success. `compile_hook` is the infallible primitive used once inputs are known
good.

## Validation grammar (invariants)

- **Action names** (`event_name`, `hook_name`): non-empty, ASCII alphanumeric plus
  `.`, `_`, `-`. This rejects whitespace, path separators and SQL metacharacters.
- **Table names** (`on_table`): exactly `schema.table` — two dot-separated parts,
  no more, no fewer. Each part is non-empty ASCII alphanumeric plus `_`. A value
  like `raw.market_data_bar;drop` fails because `;` is not allowed and there is
  only meant to be one `.` separator.
- The same `DefineEvent` always yields the same `idempotency_key`, making
  registration idempotent regardless of how many times it is replayed.
- Compilation is pure and deterministic; no global state, no I/O.
