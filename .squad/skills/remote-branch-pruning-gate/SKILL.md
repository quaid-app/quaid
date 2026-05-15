---
name: remote-branch-pruning-gate
version: 1.0
author: leela
last_updated: 2026-05-14T10:44:54.579+00:00
---

# Remote branch pruning gate

Use this when a repo has accumulated many remote branches and the team wants cleanup without deleting live work.

## Rule

Only auto-delete a remote branch when its exact tip SHA is already an ancestor of `origin/main`.

## Procedure

1. Fetch/prune first.
2. Compare each remote branch tip to `origin/main`.
3. Build two buckets:
   - **Safe delete:** branch tip already contained in `origin/main`
   - **Owner review required:** branch still ahead of `origin/main`
4. Exclude any branch tied to an active change, current release lane, or fresh operational batch even if the name looks stale.

## Why

Branch names lie; ancestry does not. A stale-looking branch can still contain unique commits, while an ugly old branch can already be fully merged and safe to remove.
