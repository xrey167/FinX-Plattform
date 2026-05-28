<!-- Parent: ../AGENTS.md -->
<!-- Generated: 2026-05-27 | Updated: 2026-05-27 -->

# scripts

## Purpose

PowerShell automation for repo-level operations that are not Cargo commands —
currently only worktree creation and teardown. These scripts are **always**
invoked from the primary checkout root, never from a worktree.

## Subdirectories

| Directory | Purpose |
|-----------|---------|
| `git/` | Local git automation. `new-worktree.ps1` creates sibling worktrees; `remove-worktree.ps1` tears them down after clean/merged verification. |

## Key Files

| File | Description |
|------|-------------|
| `git/new-worktree.ps1` | `.\scripts\git\new-worktree.ps1 -Name <topic>` creates branch `work/<topic>` and sibling worktree `FinX-Plattform-<topic>/`. |
| `git/remove-worktree.ps1` | Refuses dirty or unmerged worktrees, then removes the worktree and optionally deletes the local branch. |

## For AI Agents

### Working In This Directory

- **PowerShell only**, no bash scripts here. The repo targets Windows 11 as
  the primary dev environment.
- `new-worktree.ps1` enforces the `work/<Name>` branch convention. Do not
  bypass it with bare `git worktree add`.
- `remove-worktree.ps1` is the supported teardown path for sibling worktrees.
  Run it with `-DryRun` when auditing many old branches.
- One-off remote bootstrap (e.g., for a fork) is documented in
  `../docs/github.md` and uses `gh repo create` directly — no wrapper script
  is kept here.

## Dependencies

### External

- PowerShell 7+ (`pwsh`).
- `git` 2.5+ for worktree support.
- `gh` CLI for PR / issue operations and the one-off remote bootstrap.

<!-- MANUAL: Notes added below this line are preserved on regeneration. -->
