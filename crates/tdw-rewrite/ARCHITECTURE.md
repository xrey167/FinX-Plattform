# tdw-rewrite architecture

A single-module, dependency-free crate (`src/lib.rs`).

## Module map

| Item | Role |
|------|------|
| `CRATE_NAME: &str` | `"tdw-rewrite"`. |
| `RewriteRule` | `{ rule_id, find, replace, enabled }`. |
| `RewritePlan` | `{ rules: Vec<RewriteRule> }`. |
| `RewriteError` | `InvalidRuleId` / `EmptyFind` / `UnsafePattern`. |
| `apply_rewrites(input, plan)` | Validate, then apply enabled rules in order. |
| `validate_plan(plan)` | Validation only. |
| `is_identifier` / `contains_control_or_shell` (private) | Safety guards. |

## Contract

`apply_rewrites` runs `validate_plan` first, then folds the **enabled** rules
over the input with `str::replace`, in list order. Validation up front means an
invalid plan never partially rewrites the string.

`validate_plan` checks every rule (enabled or not) so a disabled-but-malformed
rule still fails fast rather than silently shipping: a valid identifier
`rule_id`, a non-empty `find`, and no control/shell characters (`;`, `|`,
`` ` ``) in `find` or `replace`. The shell-metacharacter guard matches
`tdw-fn-string`: rewrite plans can be authored as data, so the replacement text
is constrained to keep a crafted plan from smuggling injection payloads.

The `enabled` flag lets a deployment keep a rule's definition (and its
`rule_id`) while turning it off, instead of deleting and re-adding it.

## Invariants

- **No `unsafe`.** `#![forbid(unsafe_code)]`.
- **Validate before rewrite** — whole-plan validation precedes any replacement.
- **Deterministic & pure** — enabled rules apply in order; no side effects.
- **No control/shell injection** in `find` / `replace`.
- **No dependencies** — the crate stands alone.
- **No `unwrap` / `dbg!` / `todo!`** (workspace clippy); clean-room (no
  vendor-derived code or branding).
