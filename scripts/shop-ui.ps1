#Requires -Version 5.1
<#
.SYNOPSIS
  Starts the local shop ui binary (target\release\shop.exe ui).

.DESCRIPTION
  Shop ui is a process you start. This script does not invent a module,
  does not write C:\TextPCB Platform, and does not cargo-build a missing
  exe. Missing binary is WAIT.

.PARAMETER Port
  Listen port. Default 7745 (shop DEFAULT_PORT). Shop may bind an
  ephemeral port if this one is taken; use the printed URL.

.PARAMETER Store
  Shop store directory (global --store). Default .shop

.PARAMETER Mailbox
  Optional mailbox directory (global --mailbox).

.PARAMETER From
  Optional assign-record from field (global --from).
#>
[CmdletBinding()]
param(
    [int]$Port = 7745,
    [string]$Store = ".shop",
    [string]$Mailbox,
    [string]$From
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $repoRoot "target\release\shop.exe"

if (-not (Test-Path -LiteralPath $exe)) {
    Write-Error @"
WAIT: local shop ui binary not found:
  $exe
Run from the shop-floor repo root:
  cargo build --release
See docs/WINDOWS.md. Incomplete evidence is WAIT, never a fake PASS.
"@
    exit 2
}

$shopArgs = @()
if ($Store) {
    $shopArgs += @("--store", $Store)
}
if ($Mailbox) {
    $shopArgs += @("--mailbox", $Mailbox)
}
if ($From) {
    $shopArgs += @("--from", $From)
}
$shopArgs += @("ui", "--port", "$Port")

Set-Location -LiteralPath $repoRoot
Write-Host "starting local shop ui: $exe $($shopArgs -join ' ')"
& $exe @shopArgs
$exit = $LASTEXITCODE
if ($null -ne $exit -and $exit -ne 0) {
    Write-Error "WAIT: shop ui exited $exit (process did not stay up)"
    exit $exit
}
