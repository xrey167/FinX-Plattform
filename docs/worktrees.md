# Worktrees

Use sibling worktrees for phase work:

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
