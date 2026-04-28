---
id: protect-main-guardrails
title: "Repository guardrails: block direct pushes to main"
status: proposed
type: ops
owner: fry
reviewers: []
created: 2026-04-28
---

# Repository guardrails: block direct pushes to main

## Problem

A prior session pushed a release straight to `origin/main`, violating the repo-visible
branch + PR directive in `.squad/decisions.md` and `docs/contributing.md`. The repo
currently has no branch protection on `main`, no versioned hook bootstrap, and no
workflow dedicated to flagging a direct push if GitHub receives one.

## Change

Layer the safeguards so bypassing one control is not enough:

- add repo-versioned `pre-push` hook support that blocks pushes to protected branches
  (`main`, `master`, plus configurable extras)
- add checked-in bootstrap scripts so contributors can install the hooks per clone
- document hook installation in contributor onboarding and source-build instructions
- add `CODEOWNERS` so required code-owner review on `main` is enforceable
- add a GitHub Actions workflow that fails loudly when a push lands on `main`
  without an associated pull request
- attempt to enable GitHub branch protection on `main` requiring pull requests

## Non-Goals

- No product/runtime behaviour changes in the Quaid binary
- No replacement for GitHub branch protection with local hooks alone

## Verification

- `cargo check`
- `cargo test`
- targeted hook validation showing `.githooks/pre-push` rejects `main`
- `gh api` branch-protection attempt with exact result recorded
