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
- [x] 3.2 Attempt to enable GitHub branch protection on `main` requiring pull requests and record the exact outcome
- [x] 3.3 Add `.github/CODEOWNERS` so required code-owner review on `main` is enforceable

## 4. Validate

- [x] 4.1 Run existing repo validation commands (`cargo check`, `cargo test`)
- [x] 4.2 Run targeted hook/bootstrap validation commands and capture the results for the final handoff
