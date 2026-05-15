# Fry decision — v0.22.3 release housekeeping

- Date: 2026-05-15
- Scope: remote branch pruning, release validation, and v0.22.3 cut sequencing

## Decision

Use a strict ancestry gate for remote cleanup: only delete remote branches whose exact tip SHA is already contained in `origin/main`. Preserve release/spec/active branches and anything still ahead of `origin/main`.

For v0.22.3, the release gate is `cargo test`, and the release should only advance to tag/publish after that gate passes with the existing release-doc/version edits staged as one coherent release-bound change.

## Evidence

- `git merge-base --is-ancestor <branch-tip> origin/main` classified the safe delete set.
- `cargo test` passed for the v0.22.3 tree on 2026-05-15.
- GitHub CLI release publishing is currently unauthenticated in this environment unless `gh auth login` or `GH_TOKEN` is supplied.
