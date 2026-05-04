---
name: draft-pr-truth-refresh
version: 1.2
author: zapp
last_updated: 2026-05-04T07:22:12.881+08:00
---

# Draft PR truth refresh

Use this when a draft PR body lags behind newly pushed commits and GitHub reports `mergeable_state: dirty`.

## When to apply

- A new commit lands after the draft PR body was written
- The PR body still describes an older slice or an older "remaining work starts at" boundary
- GitHub shows `DIRTY` / `CONFLICTING` and you need to know whether that is a real merge conflict or stale metadata

## Pattern

### 1. Verify the pushed slice first

- Read the PR head SHA
- Inspect the exact head commit and the touched files/tests
- If a follow-up fix is what turned the slice from "blocked" to "approved," name that seam directly instead of repeating the older blocked status
- Update the body to name only the pushed, verified surface
- Move any "remaining work starts at" marker to the current task boundary

### 2. Separate shipped scope from non-claims

- Name the exact code paths, flags, tools, and tests that landed
- Keep an explicit list of what is still not claimed
- If branch ancestry makes the compare view broader than the landed slice, say so directly

### 3. Triage `dirty` before advising anyone

- Check GitHub's mergeability status
- Run a merge simulation against current `main`
- Recompute the exact conflict file list on each refresh; do not reuse yesterday's count, because spec-only add/add conflicts can grow or shrink while the truthful product scope stays the same
- If conflicts reproduce, report the smallest next action as "refresh from main and resolve these files"
- If conflicts do not reproduce, treat it as likely stale metadata and avoid overstating the problem

### 4. Keep the coordinator action minimal

- Do not widen the PR scope just to make the body feel complete
- Do not mark the PR ready unless that was explicitly requested
- Tell the coordinator whether the conflict is product code or docs/spec-only

## Anti-patterns

- Claiming a feature because it is proposed, not because it is on the pushed branch
- Leaving stale non-claims after a landing commit changes the true scope
- Calling `mergeable_state: dirty` "probably stale" without reproducing the merge result
