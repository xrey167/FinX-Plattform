param(
    [Parameter(Mandatory = $true)]
    [string]$Owner,

    [string]$Name = "FinX-Plattform",

    [ValidateSet("private", "public", "internal")]
    [string]$Visibility = "private"
)

$ErrorActionPreference = "Stop"

gh auth status
gh repo create "$Owner/$Name" "--$Visibility" --source . --remote origin --description "Private Rust TDW workspace for FinX-Plattform"
Write-Host "Created GitHub repository $Owner/$Name as $Visibility"
