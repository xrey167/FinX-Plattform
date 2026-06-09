<#
.SYNOPSIS
    Idempotent first-run setup for the local Docker Compose stack.

.DESCRIPTION
    Copies .env.example to .env if .env does not already exist, then fills the
    TDW_MCP_HTTP_TOKEN placeholder with a securely random hex-32 value so a
    non-loopback MCP bind starts. Re-running is safe: an existing .env is left
    untouched, and a TDW_MCP_HTTP_TOKEN that already has a non-placeholder value
    is preserved. Prints what it did.

    See docs/CONFIGURATION.md and docs/release/secrets-and-tls.md.
#>
param()

$ErrorActionPreference = "Stop"

$root = (git rev-parse --show-toplevel).Trim()
$envPath = Join-Path $root ".env"
$examplePath = Join-Path $root ".env.example"
$placeholder = "change-me-before-exposing"

function New-RandomHex32 {
    $bytes = New-Object byte[] 32
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $rng.GetBytes($bytes)
    } finally {
        $rng.Dispose()
    }
    return -join ($bytes | ForEach-Object { $_.ToString("x2") })
}

if (-not (Test-Path -LiteralPath $examplePath)) {
    throw ".env.example not found at $examplePath"
}

if (Test-Path -LiteralPath $envPath) {
    Write-Host ".env already exists at $envPath — leaving it untouched."
} else {
    Copy-Item -LiteralPath $examplePath -Destination $envPath
    Write-Host "Created .env from .env.example."
}

$lines = Get-Content -LiteralPath $envPath
$tokenLineIndex = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match '^\s*TDW_MCP_HTTP_TOKEN\s*=') {
        $tokenLineIndex = $i
        break
    }
}

if ($tokenLineIndex -lt 0) {
    $token = New-RandomHex32
    $lines += "TDW_MCP_HTTP_TOKEN=$token"
    Set-Content -LiteralPath $envPath -Value $lines
    Write-Host "Appended TDW_MCP_HTTP_TOKEN with a random hex-32 value."
} else {
    $current = ($lines[$tokenLineIndex] -split '=', 2)[1]
    if ([string]::IsNullOrWhiteSpace($current) -or $current -eq $placeholder) {
        $token = New-RandomHex32
        $lines[$tokenLineIndex] = "TDW_MCP_HTTP_TOKEN=$token"
        Set-Content -LiteralPath $envPath -Value $lines
        Write-Host "Set TDW_MCP_HTTP_TOKEN to a random hex-32 value."
    } else {
        Write-Host "TDW_MCP_HTTP_TOKEN already set to a non-placeholder value — preserved."
    }
}

Write-Host "Done. Edit .env to add provider/LLM keys, then: docker compose --profile live up -d --build"
