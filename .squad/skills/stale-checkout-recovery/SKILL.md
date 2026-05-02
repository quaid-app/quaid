---
name: "stale-checkout-recovery"
description: "How to assess and recover from a working tree parked on a stale branch before starting a new implementation batch"
domain: "git-workflow"
confidence: "high"
source: "extracted — Batch 4 pre-flight assessment, 2026-04-30T06:37:20.531+08:00"
---

## Problem

The main clone (`D:\repos\quaid`) may be parked on a stale release or feature branch
(e.g., `release/v0.11.0`) while `origin/main` has advanced by one or more merged batches.
This creates false readings:

- `tasks.md` in the local checkout shows items as **open** that are actually **closed** on `origin/main`.
- Completion percentage appears lower than reality.
- Any code work started from the stale branch will diverge from the correct base.

## Diagnostic pattern

Run this read-only check before any batch:

```powershell
# What branch are we on?
git branch --show-current

# Are we ahead/behind origin/main?
git log --oneline HEAD..origin/main   # commits on main not in HEAD
git log --oneline origin/main..HEAD   # local commits not yet on main

# What is the HEAD tag (if any)?
git tag --points-at origin/main

# Read tasks.md from origin/main, not the local checkout
git show origin/main:openspec/changes/vault-sync-engine/tasks.md | Select-String "PATTERN"
```

**If `HEAD..origin/main` returns commits:** the local checkout is behind main. Do NOT start implementation here.

## Recovery pattern

1. **Do not switch branches in the main clone** if worktrees are active (see `git-workflow` skill).
2. **Create a sibling worktree from `origin/main`:**

```powershell
cd D:\repos\quaid
git fetch origin main --tags
git worktree add ..\quaid-<change>-batch<N>-v<version> `
  -b spec/<change>-batch<N>-v<version> `
  origin/main
```

3. **Verify the starting SHA** matches the expected release tag:

```powershell
cd ..\quaid-<change>-batch<N>-v<version>
git log --oneline -1    # should show the tagged merge commit
```

4. **All implementation work happens in the sibling worktree.** The stale main clone is left as-is.

## Task status source of truth

When assessing batch completion status, ALWAYS read from `origin/main`, not from a local checkout that may be on a stale branch:

```powershell
git show origin/main:openspec/changes/vault-sync-engine/tasks.md
```

The local checkout's `tasks.md` is only authoritative when `HEAD == origin/main`.

## Dirty files in the stale checkout

Modified or untracked `.squad/` files in the stale checkout (health reports, skill files, etc.)
are Scribe artifacts and **do not block** a new batch worktree. Leave them — Scribe should
commit or clean them separately. They do not affect the new worktree's object store.

## Anti-patterns

- ❌ Starting implementation in the stale main clone (wrong base, wrong tasks.md view)
- ❌ Using `git checkout main` in the main clone when worktrees are active
- ❌ Trusting local tasks.md open/closed state without verifying against `origin/main`
- ❌ Admin-merging around reviewer gates to compensate for branch confusion
