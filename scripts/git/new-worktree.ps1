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
Write-Host "Created $worktreePath on branch $branchName"
