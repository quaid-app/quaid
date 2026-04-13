<#
.SYNOPSIS
    Creates GitHub labels and Sprint 0 / Phase 1-3 issues for GigaBrain.

.DESCRIPTION
    Requires: gh CLI (https://cli.github.com) authenticated against macro88/gigabrain.
    Run from the repo root:
        powershell -ExecutionPolicy Bypass -File .\scripts\create-labels-and-issues.ps1

.NOTES
    Labels use --force so the script is safe to re-run (existing labels are updated).
    Issues are always appended — check for duplicates before re-running.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ── Preflight ────────────────────────────────────────────────────────────────
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    Write-Error "gh CLI not found. Install from https://cli.github.com and authenticate with 'gh auth login'."
    exit 1
}

$repoRoot = git rev-parse --show-toplevel 2>$null
if (-not $repoRoot) {
    Write-Error "Not inside a Git repository. Run this from the gigabrain repo root."
    exit 1
}

Write-Host ""
Write-Host "=== GigaBrain: Create Labels & Issues ===" -ForegroundColor Cyan

# ── Labels ───────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host ">> Creating GitHub labels..." -ForegroundColor Yellow

$labels = @(
    @{ Name = "phase-1";         Color = "0075ca"; Desc = "Phase 1: Core Storage, CLI, Search, MCP" }
    @{ Name = "phase-2";         Color = "e4e669"; Desc = "Phase 2: Intelligence Layer" }
    @{ Name = "phase-3";         Color = "d93f0b"; Desc = "Phase 3: Polish, Benchmarks, Release" }
    @{ Name = "squad:fry";       Color = "bfd4f2"; Desc = "Assigned to Fry (main engineer)" }
    @{ Name = "squad:bender";    Color = "bfd4f2"; Desc = "Assigned to Bender (tester)" }
    @{ Name = "squad:professor"; Color = "bfd4f2"; Desc = "Assigned to Professor (code reviewer)" }
    @{ Name = "squad:nibbler";   Color = "bfd4f2"; Desc = "Assigned to Nibbler (adversarial reviewer)" }
    @{ Name = "squad:kif";       Color = "bfd4f2"; Desc = "Assigned to Kif (benchmarks)" }
    @{ Name = "squad:zapp";      Color = "bfd4f2"; Desc = "Assigned to Zapp (DevRel/release)" }
)

foreach ($lbl in $labels) {
    Write-Host "  label: $($lbl.Name)"
    gh label create $lbl.Name --color $lbl.Color --description $lbl.Desc --force
}

Write-Host "  Done ($($labels.Count) labels)." -ForegroundColor Green

# ── Issues ───────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host ">> Creating GitHub issues..." -ForegroundColor Yellow

$issues = @(
    @{
        Title  = "[Sprint 0] Repository scaffold + CI/CD"
        Body   = "Tracks the Sprint 0 scaffold work.`nOpenSpec: openspec/changes/sprint-0-repo-scaffold/proposal.md"
        Labels = "squad:fry"
    }
    @{
        Title  = "[Phase 1] Core storage, CLI, search, MCP"
        Body   = "Fry implements Phase 1.`nOpenSpec: openspec/changes/p1-core-storage-cli/proposal.md`nGate: round-trip tests pass; MCP connects; static binary verified."
        Labels = "squad:fry,phase-1"
    }
    @{
        Title  = "[Phase 1] Round-trip test + ship gate sign-off"
        Body   = "Bender validates the Phase 1 ship gate before Phase 2 can begin."
        Labels = "squad:bender,phase-1"
    }
    @{
        Title  = "[Phase 1] Code review: db.rs, search.rs, inference.rs"
        Body   = "Professor reviews the three critical Phase 1 modules."
        Labels = "squad:professor,phase-1"
    }
    @{
        Title  = "[Phase 1] Adversarial review: MCP server"
        Body   = "Nibbler adversarially reviews the MCP server for OCC enforcement and injection risks."
        Labels = "squad:nibbler,phase-1"
    }
    @{
        Title  = "[Phase 2] Intelligence layer"
        Body   = "Fry implements Phase 2.`nOpenSpec: openspec/changes/p2-intelligence-layer/proposal.md`nBlocked until Phase 1 gate passes."
        Labels = "squad:fry,phase-2"
    }
    @{
        Title  = "[Phase 3] Benchmarks + release gates"
        Body   = "Kif establishes BEIR baseline and all release gates.`nOpenSpec: openspec/changes/p3-polish-benchmarks/proposal.md"
        Labels = "squad:kif,phase-3"
    }
    @{
        Title  = "[Phase 3] v0.1.0 release"
        Body   = "Zapp coordinates the v0.1.0 GitHub Release after Phase 3 gates pass."
        Labels = "squad:zapp,phase-3"
    }
)

foreach ($iss in $issues) {
    Write-Host "  issue: $($iss.Title)"
    gh issue create --title $iss.Title --body $iss.Body --label $iss.Labels
}

Write-Host ""
Write-Host "  Done ($($issues.Count) issues)." -ForegroundColor Green
Write-Host ""
Write-Host "=== Complete ===" -ForegroundColor Cyan
Write-Host ""
