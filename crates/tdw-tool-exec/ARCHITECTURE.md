# tdw-tool-exec architecture

A single-module crate (`src/lib.rs`): the Phase-1 tool-execution backend that
resolves a `tdw-agent` registry tool's implementation and dispatches it.

## Module map

| Item | Role |
|------|------|
| `ToolExecutor` | Holds an in-process `tdw-tools::ToolRegistry` of `Builtin` handlers, a `CommandPolicy`, and a `SchemaValidation` mode; `execute(...)` validates (if opted in) then dispatches. |
| `CommandPolicy` | Deny-by-default `Command` allow-list + timeout (`new`, `from_env`, `default`, `authorize`). |
| `SchemaValidation` | Opt-in arg-validation toggle (`Off` default / `On`); `from_env` reads `TDW_TOOL_EXEC_VALIDATE_ARGS`, `parse` is the env-free core. |
| `ToolOutcome` | `{ structured: Value }`. |
| `ExecError` | `Unbound` / `ToolNotFound` / `HandlerNotFound` / `NotPermitted` / `NotYetSupported` / `Backend` / `BadArguments` / `InvalidArguments`. |
| `validate_command` / `dispatch_command` / `spawn_reader` / `join_reader` (private) | The hardened command path. |
| `ReceiptLog` / `ToolReceipt` / `ChainStatus` / `ChainBreak` (`src/receipt.rs`) | Opt-in, append-only, hash-chained log of successful executions; `verify` walks the chain. Integrity-not-cryptographic (std `DefaultHasher`). |
| `validate_args` / `check_type` / `json_type_name` (private) | The opt-in argument-schema validator. |

## Dispatch contract

`ToolExecutor::execute(registry, name, args)`:

1. look up the `tool` resource in the `tdw-agent` `Registry`
   (`ToolNotFound` if absent) and re-type it to a `tdw_agent::Tool`
   (`BadArguments` if malformed);
2. if `SchemaValidation::On`, validate `args` against `tool.input_schema`
   *before* any dispatch (`InvalidArguments { tool, reason }` on mismatch); when
   `Off` (the default) this step is skipped entirely;
3. match `tool.implementation`:

| Variant | Behavior |
|---------|----------|
| `Unbound` | `Err(Unbound)` — listed but not runnable (MCP maps this to `-32601`). |
| `Builtin { handler }` | Look up `handler` in the in-process registry (`HandlerNotFound` if missing) and call it; backend errors become `Backend`. |
| `Command { background: false }` | Run via the hardened command path under `CommandPolicy`. |
| `Command { background: true }` | `NotYetSupported("background command (Phase 2)")`. |
| `Http` / `Mcp` / `Pty` / `Wasm` / `Ref` | `NotYetSupported(...)` — deferred (no credential wiring / server resolution yet). |

The deferrals are deliberate and *honest*: a fresh backend has no credential
store or MCP server resolver, so `Http`/`Mcp` cannot run yet — returning
`NotYetSupported` is truthful rather than faking success.

## Argument-schema validation (opt-in)

`SchemaValidation` defaults to `Off` (and `from_env` defaults `Off` when
`TDW_TOOL_EXEC_VALIDATE_ARGS` is unset), so an executor that does not opt in
behaves exactly as before. When `On`, `validate_args(&tool.input_schema, args)`
runs after resolution and before dispatch:

- a non-object schema (e.g. `true`/null/missing) is accepted (nothing to check);
- the root `type` is enforced (defaulting to `"object"` per the `Tool`
  contract); only object schemas carry `required`/`properties` semantics;
- each name in `required` must be present and non-null (a `null` value counts as
  missing);
- each present key listed in `properties` is checked against its declared
  `type` (`integer` = i64/u64, `number` = any JSON number, etc.); unknown type
  names and unknown keywords are ignored;
- extra keys not in `properties` are allowed (open-world; no
  `additionalProperties` enforcement).

The validator is pure, allocation-light, and **never panics** on untrusted
JSON; a failure becomes `ExecError::InvalidArguments { tool, reason }` with a
human-readable `reason`. Compound/`$ref`/`enum`/`format` keywords are out of
scope by design — this avoids both a heavyweight schema dependency and
false-positive rejections of real traffic.

## Hardened command path

`Command { background: false }` is the only variant that touches the OS, and it
is defended in depth:

- **`validate_command`** rejects anything that is not a bare program name:
  empty, containing `..`, or containing `/ \ ; & | < > ` ` $`, control
  characters, or whitespace → `BadArguments`. This runs *before* the allow-list.
- **`CommandPolicy::authorize`** is deny-by-default: `allowed = None` denies all
  execution; otherwise only exact bare names on the list pass (`NotPermitted`).
  `from_env` reads `TDW_TOOL_EXEC_ALLOWED_COMMANDS` (unset/empty = `None` = deny
  all) and `TDW_TOOL_EXEC_TIMEOUT_SECS` (default 30; `0` rejected).
- **argv is registry-controlled.** The command `args` come from the tool
  definition, not the request, so a caller cannot influence argv. (Allow-listing
  a shell interpreter trusts whoever can author tool definitions — a registry-
  trust boundary, documented in-source, not a remote bypass.)
- **Execution** spawns the child with null stdin and piped stdout/stderr, drains
  each pipe on a dedicated thread (avoiding pipe-buffer deadlock), polls for exit,
  and kills the child on timeout (`Backend("command timed out")`). On Windows it
  sets `CREATE_NO_WINDOW` to avoid a console flash.

## Invariants

- **No `unsafe`** — `#![forbid(unsafe_code)]`, even in tests (which inject an
  explicit `CommandPolicy` instead of the edition-2024 `unsafe` `set_var`, also
  making them race-free).
- **Deny-by-default commands**; bare-name-only; argv from the registry.
- **Honest deferral** for unimplemented variants; `Unbound` is distinct from
  "not yet supported".
- **Opt-in, fail-soft validation**: default `Off`, lenient subset, never panics;
  enabling it can only turn a would-be backend call into a structured
  `InvalidArguments` — it never changes a successful call's result.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
- **Receipt log is opt-in, append-only, std-hash integrity.** Off by default
  (zero behavioral change); no removal/mutation API; tamper-EVIDENT, not
  cryptographic; not `Sync` when enabled (interior mutability keeps
  `execute(&self, ...)` unchanged).
