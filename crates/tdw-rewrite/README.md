# tdw-rewrite

An ordered, validated find/replace rewrite engine with per-rule enable flags.

`tdw-rewrite` applies a `RewritePlan` (a list of `RewriteRule`s) to an input
string, running only the enabled rules in order. It mirrors `tdw-fn-string`'s
safety posture but is rule-oriented: each rule has a stable `rule_id` and can be
toggled without removing it.

## What it provides

- `RewriteRule` — `{ rule_id, find, replace, enabled }`.
- `RewritePlan` — `{ rules: Vec<RewriteRule> }`.
- `apply_rewrites(input, plan)` — validate then run, returning
  `Result<String, RewriteError>`.
- `validate_plan(plan)` — validate without running.
- `RewriteError` — `InvalidRuleId` / `EmptyFind` / `UnsafePattern`.

## Feature flags

None. The crate has **no dependencies**.

## Quickstart

```rust
use tdw_rewrite::{apply_rewrites, RewritePlan, RewriteRule};

let plan = RewritePlan {
    rules: vec![
        RewriteRule {
            rule_id: "normalize-symbol".to_string(),
            find: "aapl".to_string(),
            replace: "AAPL".to_string(),
            enabled: true,
        },
        RewriteRule {
            rule_id: "disabled".to_string(),
            find: "AAPL".to_string(),
            replace: "MSFT".to_string(),
            enabled: false, // skipped
        },
    ],
};

assert_eq!(apply_rewrites("research aapl", &plan), Ok("research AAPL".to_string()));
```

See [`examples/basic.rs`](examples/basic.rs):

```sh
cargo run -p tdw-rewrite --example tdw_rewrite_basic
```

## Safety rules

`validate_plan` (run by `apply_rewrites` first) rejects:

- a `rule_id` that is not `[A-Za-z0-9_-]+` → `InvalidRuleId`;
- an empty `find` → `EmptyFind`;
- a `find` or `replace` containing a control character or a shell metacharacter
  (`;`, `|`, `` ` ``) → `UnsafePattern`.

Only rules with `enabled: true` are applied, in list order, via `str::replace`.

## Invariants

- `#![forbid(unsafe_code)]`.
- **Validate before rewrite** — the whole plan is checked before any rule runs,
  so an invalid plan never partially rewrites the input.
- **Deterministic** — enabled rules apply in order; pure function of input + plan.
- **No shell/control injection** in `find` / `replace`.
- Workspace lints deny `unwrap` / `dbg!` / `todo!`.

See [ARCHITECTURE.md](ARCHITECTURE.md).
