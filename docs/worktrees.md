# Worktrees

Use sibling worktrees for feature, cleanup, and phase work:

```powershell
.\scripts\git\new-worktree.ps1 -Name phase-01-core
```

This creates a branch named `work/phase-01-core` and a sibling directory named
`FinX-Plattform-phase-01-core`.

Recommended branch naming:

- `work/phase-00-bootstrap`
- `work/phase-01-core`
- `work/phase-02-storage`
- `work/layer-e-event-spine`
- `review/<topic>`

Keep one logical task per branch. Before merging, run:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p xtask -- clean-room-audit
```

After a PR is merged, remove the sibling worktree through the guarded helper:

```powershell
.\scripts\git\remove-worktree.ps1 -Path ..\FinX-Plattform-phase-01-core -RemoveBranch
```

The helper refuses to remove a worktree unless:

- the worktree is clean,
- it is not the primary checkout,
- the branch is already an ancestor of `main`, or every branch commit is
  patch-equivalent to `main` according to `git cherry main <branch>`.

Use `-DryRun` first when auditing many stale worktrees.
