<!-- Parent: ../AGENTS.md -->
<!-- Generated: 2026-05-27 | Updated: 2026-05-27 -->

# scripts

## Purpose

PowerShell automation for repo-level operations that are not Cargo commands:
worktree creation and the one-shot GitHub remote bootstrap. These scripts are
**always** invoked from the primary checkout root, never from a worktree.

## Subdirectories

| Directory | Purpose |
|-----------|---------|
| `git/` | Local git automation. `new-worktree.ps1` creates sibling worktrees; `remove-worktree.ps1` tears them down after clean/merged verification. |
| `github/` | GitHub-side operations. `create-private-repo.ps1` provisioned the remote (now public; name is historical). |

## Key Files

| File | Description |
|------|-------------|
| `git/new-worktree.ps1` | `.\scripts\git\new-worktree.ps1 -Name <topic>` creates branch `work/<topic>` and sibling worktree `FinX-Plattform-<topic>/`. |
| `git/remove-worktree.ps1` | Refuses dirty or unmerged worktrees, then removes the worktree and optionally deletes the local branch. |
| `github/create-private-repo.ps1` | One-shot remote bootstrap. Now used with `-Visibility public`. |

## For AI Agents

### Working In This Directory

- **PowerShell only**, no bash scripts here. The repo targets Windows 11 as
  the primary dev environment.
- `new-worktree.ps1` enforces the `work/<Name>` branch convention. Do not
  bypass it with bare `git worktree add`.
- `remove-worktree.ps1` is the supported teardown path for sibling worktrees.
  Run it with `-DryRun` when auditing many old branches.
- `create-private-repo.ps1` is destructive at the GitHub-API layer (it can
  create or change repo visibility). Do not run it without the user's
  explicit approval — the remote already exists.

## Dependencies

### External

- PowerShell 7+ (`pwsh`).
- `git` 2.5+ for worktree support.
- `gh` CLI for `create-private-repo.ps1`.

<!-- MANUAL: Notes added below this line are preserved on regeneration. -->
