param(
    [Parameter(Mandatory = $true)]
    [string]$Path,

    [string]$Base = "main",

    [switch]$RemoveBranch,

    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Args,

        [switch]$AllowFailure
    )

    $output = & git @Args 2>&1
    $exitCode = $LASTEXITCODE
    if ($exitCode -ne 0 -and -not $AllowFailure) {
        throw "git $($Args -join ' ') failed with exit $exitCode`n$output"
    }

    [pscustomobject]@{
        ExitCode = $exitCode
        Output = @($output)
    }
}

$worktreeList = (Invoke-Git -Args @("worktree", "list", "--porcelain")).Output
$primaryLine = @($worktreeList | Where-Object { $_ -like "worktree *" } | Select-Object -First 1)
if ($primaryLine.Count -eq 0) {
    throw "Unable to identify primary worktree from git worktree list"
}
$primaryRoot = (Resolve-Path -LiteralPath $primaryLine[0].Substring(9)).Path
$repoParent = Split-Path -Parent $primaryRoot

$resolvedInput = (Resolve-Path -LiteralPath $Path).Path
$worktreeRoot = (
    Invoke-Git -Args @("-C", $resolvedInput, "rev-parse", "--show-toplevel")
).Output[0]
$worktreeRoot = (Resolve-Path -LiteralPath $worktreeRoot).Path

if ($worktreeRoot.Equals($primaryRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove the primary checkout: $worktreeRoot"
}

$repoParentPrefix = $repoParent.TrimEnd("\", "/") + [System.IO.Path]::DirectorySeparatorChar
if (-not $worktreeRoot.StartsWith($repoParentPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove worktree outside repo parent $repoParent`: $worktreeRoot"
}

$status = (Invoke-Git -Args @("-C", $worktreeRoot, "status", "--porcelain")).Output
if ($status.Count -gt 0) {
    throw "Refusing to remove dirty worktree $worktreeRoot`n$($status -join "`n")"
}

$branchResult = Invoke-Git -Args @("-C", $worktreeRoot, "symbolic-ref", "--short", "HEAD") -AllowFailure
if ($branchResult.ExitCode -ne 0 -or $branchResult.Output.Count -eq 0) {
    throw "Refusing to remove detached worktree without a branch: $worktreeRoot"
}
$branch = $branchResult.Output[0]

$ancestor = Invoke-Git -Args @("merge-base", "--is-ancestor", $branch, $Base) -AllowFailure
$verifiedBy = $null
if ($ancestor.ExitCode -eq 0) {
    $verifiedBy = "ancestor of $Base"
} else {
    $cherry = (Invoke-Git -Args @("cherry", $Base, $branch)).Output
    $unmatched = @($cherry | Where-Object { $_.TrimStart().StartsWith("+") })
    if ($unmatched.Count -gt 0) {
        $details = $unmatched -join "`n"
        throw "Refusing to remove $branch; commits are not merged or patch-equivalent to $Base`n$details"
    }
    $verifiedBy = "patch-equivalent to $Base via git cherry"
}

Write-Host "Verified $worktreeRoot on ${branch}: clean and $verifiedBy"

if ($DryRun) {
    Write-Host "Dry run only; no worktree or branch was removed."
    return
}

Invoke-Git -Args @("worktree", "remove", $worktreeRoot) | Out-Null
Write-Host "Removed worktree $worktreeRoot"

if ($RemoveBranch) {
    Invoke-Git -Args @("branch", "-d", $branch) | Out-Null
    Write-Host "Deleted local branch $branch"
}

Invoke-Git -Args @("fetch", "--prune") | Out-Null
