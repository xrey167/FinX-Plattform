param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-zA-Z0-9][a-zA-Z0-9._-]{1,80}$')]
    [string]$Name,

    [string]$Base = "main"
)

$ErrorActionPreference = "Stop"
$root = (git rev-parse --show-toplevel).Trim()
$repoName = Split-Path -Leaf $root
$parent = Split-Path -Parent $root
$branchName = "work/$Name"
$worktreePath = Join-Path $parent "$repoName-$Name"

if (Test-Path -LiteralPath $worktreePath) {
    throw "Worktree path already exists: $worktreePath"
}

git fetch --all --prune
git worktree add -b $branchName $worktreePath $Base

# Wire the multi-session pre-push guardrail (see docs/multi-session.md). This is
# repo-local config shared across all worktrees of the clone, so setting it once
# is enough; re-running is idempotent.
git config core.hooksPath .githooks

Write-Host "Created $worktreePath on branch $branchName"
