---
id: protect-main-guardrails
title: "Repository guardrails: block direct pushes to main"
status: in-revision
type: ops
owner: leela
reviewers: []
created: 2026-04-28
revised: 2026-04-28
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

## Revision note (2026-04-28 — Leela)

Professor rejected the workflow after `f20c503` for a real bypass in task 3.1:
`/commits/{sha}/pulls` returns any PR that *contains* the commit, including open PRs.
A direct push of a commit already on an open PR branch would pass the old check.

Fix applied: the workflow now filters the API results to only count PRs where all
four conditions hold simultaneously — `state == 'closed'`, `merged_at` is set,
`base.ref == 'main'`, **and** `merge_commit_sha == sha`. The last condition is the
critical gate: for a legitimate merge (regular, squash, or rebase), GitHub always sets
`merge_commit_sha` to the exact commit SHA that lands on `main`. A commit pushed
directly cannot satisfy this because `merge_commit_sha` either belongs to a future
merge commit or to a GitHub-synthesised test-merge ref, never to the raw branch commit.

## Verification

- `cargo check`
- `cargo test`
- targeted hook validation showing `.githooks/pre-push` rejects `main`
- `gh api` branch-protection attempt (pending — outcome not yet recorded; task 3.2 remains open)
- reasoned test cases for the revised 3.1 detection logic (see tasks.md 3.1 closure note)