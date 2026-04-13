<#
.SYNOPSIS
    Creates the sprint-0/scaffold branch, stages all files, commits, and pushes.

.DESCRIPTION
    Requires: git configured with push access to origin (macro88/gigabrain).
    Run from the repo root:
        powershell -ExecutionPolicy Bypass -File .\scripts\push-sprint0-branch.ps1

.NOTES
    Safety: refuses to run if already on sprint-0/scaffold or if the branch already exists.
    Never touches main — creates a new branch and pushes it to origin.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$BranchName = "sprint-0/scaffold"

# ── Preflight ────────────────────────────────────────────────────────────────
$repoRoot = git rev-parse --show-toplevel 2>$null
if (-not $repoRoot) {
    Write-Error "Not inside a Git repository. Run this from the gigabrain repo root."
    exit 1
}

$currentBranch = git branch --show-current
if ($currentBranch -eq $BranchName) {
    Write-Host "Already on branch '$BranchName'. Skipping branch creation." -ForegroundColor Yellow
}
else {
    # Check if branch already exists locally
    $existingBranch = git branch --list $BranchName
    if ($existingBranch) {
        Write-Error "Branch '$BranchName' already exists locally. Switch to it manually if needed: git checkout $BranchName"
        exit 1
    }

    Write-Host "Creating branch: $BranchName" -ForegroundColor Cyan
    git checkout -b $BranchName
}

# ── Stage ────────────────────────────────────────────────────────────────────
Write-Host "Staging all files..." -ForegroundColor Yellow
git add .

# Show what will be committed
Write-Host ""
Write-Host ">> Files staged:" -ForegroundColor Yellow
git diff --cached --stat

# ── Commit ───────────────────────────────────────────────────────────────────
$commitMessage = @"
Sprint 0: repository scaffold, CI/CD, OpenSpec proposals

- Cargo.toml with full dependency declarations
- src/ module stubs (commands, core, mcp)
- src/schema.sql — full v4 DDL
- skills/ stubs for all 8 skills
- tests/fixtures, benchmarks/README.md
- CLAUDE.md, AGENTS.md
- .github/workflows/ci.yml and release.yml
- openspec/changes/ proposals for all 4 phases
- .squad/ team configuration and decisions

Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>
"@

Write-Host ""
Write-Host "Committing..." -ForegroundColor Yellow
git commit -m $commitMessage

# ── Push ─────────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host "Pushing to origin/$BranchName..." -ForegroundColor Yellow
git push origin $BranchName

Write-Host ""
Write-Host "=== Done ===" -ForegroundColor Green
Write-Host ""
Write-Host "Next steps:" -ForegroundColor Cyan
Write-Host "  1. Open a PR from '$BranchName' to 'main' on GitHub"
Write-Host "  2. Use the PR body template in .squad/decisions/inbox/leela-ops-pass.md"
Write-Host "  3. Link the PR to the Sprint 0 issue created by create-labels-and-issues.ps1"
Write-Host ""
