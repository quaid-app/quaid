<#
.SYNOPSIS
    Installs the repo-versioned Git hooks for this clone.

.DESCRIPTION
    Configures git to use .githooks as core.hooksPath and seeds the protected-branch
    list if it has not been set already.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = git rev-parse --show-toplevel 2>$null
if (-not $repoRoot) {
    throw "Run this from inside the quaid repository."
}

Set-Location $repoRoot

$hookPath = Join-Path $repoRoot ".githooks\pre-push"
if (-not (Test-Path $hookPath)) {
    throw "Expected hook at $hookPath"
}

git config --local core.hooksPath .githooks

$protectedBranches = git config --local --get quaid.protectedBranches 2>$null
if (-not $protectedBranches) {
    git config --local quaid.protectedBranches "main master"
    $protectedBranches = "main master"
}

Write-Host "Installed repo hooks:" -ForegroundColor Green
Write-Host ("  core.hooksPath={0}" -f (git config --local --get core.hooksPath))
Write-Host ("  quaid.protectedBranches={0}" -f $protectedBranches)
Write-Host "Protected-branch pushes to main/master now fail locally before Git contacts origin."
