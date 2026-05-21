# Contributing

## Branches And Worktrees

Use short-lived feature branches. Prefer the worktree helper:

```powershell
.\scripts\git\new-worktree.ps1 -Name phase-01-core
```

Worktrees are created as siblings of this repository so generated build output and
parallel edits do not collide.

## Required Local Checks

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p xtask -- clean-room-audit
```

## Clean-Room Rule

Do not copy code, trait signatures, or tests from FinX-XR. FinX-XR may be read only
for high-level pattern awareness when a plan explicitly calls for it.
