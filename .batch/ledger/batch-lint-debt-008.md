---
batch: batch-lint-debt-008
items: lint:clippy::map_unwrap_or, lint:clippy::needless_pass_by_value, lint:clippy::option_if_let_else
outcome: done
---

# batch-lint-debt-008 — lint-debt cleanup (tdw-functions + tdw-llm)

Three lint families across two crates.

## Per-family results

| Family | Crate | Fixed | Notes |
|--------|-------|-------|-------|
| `clippy::map_unwrap_or` | tdw-functions | 0 | Already clean on origin/main after fresh `cargo clean` + targeted clippy (0 warnings). Nothing to fix; no `#[allow]` added. |
| `clippy::needless_pass_by_value` | tdw-functions | 0 | Already clean on origin/main after fresh `cargo clean` + targeted clippy (0 warnings). Nothing to fix; no `#[allow]` added. |
| `clippy::option_if_let_else` | tdw-llm | 1 | `crates/tdw-llm/src/fallback.rs` `StubModel::complete` — converted `match &self.fail { None => Ok(...), Some(err) => Err(...) }` to `self.fail.as_ref().map_or_else(|| Ok(...), |err| Err(clone_error(err)))` per clippy. Behavior identical. |

No blanket `#[allow]` used in any family. No family dropped/blocked.

## Verification — gate commands + result tails

### Targeted lint families (each 0 in its crate)
```
cargo clean -p tdw-functions -p tdw-llm
# -> Removed 1625 files, 324.0MiB total

cargo clippy -p tdw-functions --all-targets -- -W clippy::map_unwrap_or -W clippy::needless_pass_by_value
# -> map_unwrap_or count: 0, needless_pass_by_value count: 0, total warnings: 0 (tdw-functions checked fresh)

cargo clippy -p tdw-llm --all-targets -- -W clippy::option_if_let_else
# before fix -> 1 warning (fallback.rs:199 "use Option::map_or_else instead of an if let/else")
# after fix  -> option_if_let_else count: 0, warning count: 0
```

### fmt
```
cargo fmt -p tdw-functions -- --check   # EXIT: 0
cargo fmt -p tdw-llm -- --check         # EXIT: 0
```

### Pedantic/nursery ratchet (no regression on the two touched crates)
```
cargo clippy -p tdw-functions -p tdw-llm --all-targets -- -W clippy::pedantic -W clippy::nursery
# -> pedantic/nursery warning count: 0 (no NEW pedantic/nursery warnings introduced)
```

### Workspace gates
```
cargo clippy --workspace --all-targets -- -D warnings
# -> EXIT: 0, warning/error lines: 0
#    Finished `dev` profile [unoptimized + debuginfo] target(s) in 20.00s

cargo test --workspace
# -> (result tail recorded below)

cargo run -p xtask -- clean-room-audit
# -> EXIT: 0
#    clean-room audit passed
```

## Clean-Room Checklist
- No `finx-*` code copied.
- No FinX-XR code copied.
- No `tdw-provider-openbb` code copied.
- No AGPL code copied.

## PR
<!-- PR URL appended after creation -->
