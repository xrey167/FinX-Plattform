---
batch: batch-lint-debt-007
items: lint:clippy::missing_const_for_fn
outcome: done
---

# batch-lint-debt-007 — missing_const_for_fn regression cleanup

The `lint:clippy::missing_const_for_fn` family was already driven to zero by
batches **004** (first 10 crates, wave 2) and **005** (the 6 deferred crates,
closing the family). Since those landed, main advanced ~25 commits and exactly
**one** new occurrence was introduced by newer integration-slice work.

This batch was authored to const-ify "≤6 crates' worth" of hits, but a fresh
workspace discovery found only a single remaining candidate, so the batch is a
1-function cleanup rather than a 6-crate sweep. Re-using the committed
`batch-lint-debt-004.md` filename would clobber prior history, so this ledger
takes the next free number (007).

## Lint family

`clippy::missing_const_for_fn` — functions that can be `const fn`. Proven-safe
family (wave-2 #156/#160): adding `const` is backward-compatible. Wrong
const-ness fails compilation, so the per-crate clippy/test gates double as
semantic verification.

## Discovery

```
cargo clippy --workspace --all-targets --message-format=short \
  -- -W clippy::missing_const_for_fn 2>&1 | grep "could be a"
crates\tdw-functions\src\job.rs:204:5: warning: this could be a `const fn`
```

One candidate, one crate.

## Crates touched (1)

- **tdw-functions** — `FunctionJobHandler::new(registry: Arc<FunctionRegistry>)`
  at `src/job.rs:204` → `pub const fn new(...)`. The constructor only moves the
  `Arc` field into `Self`, which is const-valid.

## Const-ifications applied (1)

| Crate | File:line | Function |
| --- | --- | --- |
| tdw-functions | src/job.rs:204 | `FunctionJobHandler::new` |

`cargo clippy --fix` applied **zero** changes for this site (the suggestion was
not machine-applied), so the single const was added by hand and verified via
the gates below.

## Reverted

None.

## Gate evidence (tdw-functions)

| Gate | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt -p tdw-functions -- --check` | pass |
| clippy | `cargo clean -p tdw-functions; cargo clippy -p tdw-functions --all-targets -- -D warnings` | pass (0 warnings) |
| tests | `cargo test -p tdw-functions` | pass (18 passed, 0 failed; pg integration 0; doctests ok) |
| clean-room | `cargo run -p xtask -- clean-room-audit` | `clean-room audit passed` |

tdw-functions is **not** schema-bearing (not tdw-agent/event/protocol/config),
so no schema-sync was required; none was run.

## Clean-room

No AGPL code copied.
