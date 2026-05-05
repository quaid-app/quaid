---
change: protect-main-guardrails
---

# Tasks

## 1. Add repo-versioned protected-branch hooks

- [x] 1.1 Add `.githooks/pre-push` that blocks pushes to `main`, `master`, and configured protected branches
- [x] 1.2 Add checked-in setup scripts for shell and PowerShell contributors to install `core.hooksPath`
- [x] 1.3 Add a PR-time CI verification step that ensures the hook bootstrap works and the hook rejects protected pushes

## 2. Update contributor bootstrap docs

- [x] 2.1 Update source-build docs to install the repo hooks immediately after clone
- [x] 2.2 Update contributing workflow docs so hooks are a required setup step before branching and PR work

## 3. Add GitHub-side detection and protection

- [x] 3.1 Add a workflow that fails when GitHub receives a push to `main` with no associated pull request
  > **Revised (Leela, 2026-04-28):** Original implementation had a real bypass — `/commits/{sha}/pulls` returns PRs that *contain* the commit, including open/unmerged PRs, so a direct push of a commit already on an open PR branch would slip through. Fix: filter API results to PRs where `state=='closed'` AND `merged_at` is set AND `base.ref=='main'` AND `merge_commit_sha==sha`. The `merge_commit_sha` check is the load-bearing gate: GitHub sets this to the exact commit SHA that lands on `main` for all merge strategies (regular, squash, rebase). A directly-pushed commit cannot match because its SHA is not a merge commit.
- [ ] 3.2 Attempt to enable GitHub branch protection on `main` requiring pull requests and record the exact outcome
- [x] 3.3 Add `.github/CODEOWNERS` so required code-owner review on `main` is enforceable

## 4. Validate

- [x] 4.1 Run existing repo validation commands (`cargo check`, `cargo test`)
- [x] 4.2 Run targeted hook/bootstrap validation commands and capture the results for the final handoff
