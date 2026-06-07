# tdw-define

`DEFINE EVENT ... ON <table>` declarations that compile to executable hook
specifications. This is the declarative front end that turns a table-change
trigger into a `tdw-hooks` [`HookSpec`].

## Purpose

A `DefineEvent` is a small, validated record describing "when something happens
on this table, fire this hook". It is the platform's analogue of a database
event/trigger declaration:

- `event_name` — the logical event being declared.
- `on_table` — the `schema.table` it watches.
- `hook_name` — the hook to fire.
- `transaction_mode` — when the hook runs relative to the commit boundary
  (`InTransaction`, `PostCommit`, `Rollback`), reusing `tdw_hooks::TransactionMode`.

It compiles to a `HookSpec` (`compile_hook`) and produces a stable
`idempotency_key` so the same declaration is never registered twice. All inputs
are validated against safe identifier/table-name grammars to prevent injection.

The crate is pure: `#![forbid(unsafe_code)]`, no I/O, no async.

## Feature flags

None.

## Dependencies

- `serde` — (de)serialization of `DefineEvent` / `DefineError`.
- `tdw-hooks` — provides `HookSpec` and `TransactionMode`.

## Quickstart

```rust
use tdw_define::DefineEvent;
use tdw_hooks::TransactionMode;

let define = DefineEvent {
    event_name: "market_data_changed".to_string(),
    on_table: "raw.market_data_bar".to_string(),
    hook_name: "emit.market_data_changed".to_string(),
    transaction_mode: TransactionMode::PostCommit,
};

// Validate + compile in one step.
let hook = define.try_compile_hook().expect("valid declaration");
assert_eq!(hook.name, "emit.market_data_changed");

// Stable idempotency key: on_table:event_name:hook_name.
assert_eq!(
    define.idempotency_key(),
    "raw.market_data_bar:market_data_changed:emit.market_data_changed"
);
```

Run the worked example:

```text
cargo run -p tdw-define --example basic
```

## See also

- [`ARCHITECTURE.md`](./ARCHITECTURE.md) — validation grammar and compile contract.
- `tdw-hooks` — the `HookSpec`/`HookRegistry` runtime this compiles into.
