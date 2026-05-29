---
name: pick-worktree
description: List all FinX-Plattform sibling worktrees with branch, last commit, and dirty state, then change context to the chosen one. Use when the user asks "which worktree", "switch to <topic>", "what's on g0NN-*", or before running cargo verification commands.
---

# pick-worktree

The workspace root `C:/Users/ReyDa/FinX-Finance/` is **not** a git repo. Real work happens in `FinX-Plattform/` (primary) or one of the `FinX-Plattform-<topic>/` sibling worktrees on `work/<topic>` branches.

## Steps

1. **List worktrees** by running, from any path inside any worktree:
   ```
   git worktree list --porcelain
   ```
   Parse the output into rows: `path | branch`.

2. **Enrich each row** in parallel:
   - `git -C <path> log -1 --format='%h %s (%cr)'` — last commit
   - `git -C <path> status --porcelain | wc -l` — dirty file count
   - `git -C <path> rev-list --count <branch>...origin/main` — commits ahead of main

3. **Render a table**:
   ```
   PATH                              BRANCH          AHEAD  DIRTY  LAST COMMIT
   FinX-Plattform                    main            0      0      abc123 docs: ... (2h ago)
   FinX-Plattform-g010-ch            work/g010-ch    3      2      ...
   ```

4. **If the user named a topic**, confirm which worktree maps to it (`g010-ch` → `FinX-Plattform-g010-ch`). Otherwise ask via `AskUserQuestion` with the top 4 most-recently-touched worktrees as options.

5. **Switch context** by:
   - Setting `cwd` for subsequent shell commands to the chosen worktree path.
   - Re-reading that worktree's `AGENTS.md` (it mirrors operational rules and may diverge from `main`).
   - Reminding the user: build artifacts under `target/` and `CARGO_TARGET_DIR` are **per-worktree** — see project memory `[Project Environment]`.

6. **Refuse** to operate on the workspace root `FinX-Finance` itself; it has no git state.

## Helpful patterns

- New worktree: `scripts/git/new-worktree.ps1 <topic>` (PowerShell 7+ required).
- Tear down: `git worktree remove <path> && git branch -d work/<topic>` (only after PR merged).
- Per-worktree CARGO_TARGET_DIR avoids Defender thrash on `E:\cargo-target`; keep the override scoped, do not export it globally.
