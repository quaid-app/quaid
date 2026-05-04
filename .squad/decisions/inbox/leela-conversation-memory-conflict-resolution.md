# Leela — conversation-memory conflict resolution

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **PR:** #153 (`feat/slm-conversation-mem`)
- **Scope:** Resolve six OpenSpec add/add conflicts against `main`

## Decision

Keep the conflict resolutions on the truth-repaired branch versions of the six `conversation-memory-foundations` artifacts.

## Why

`main` carries earlier draft copies of the same change that still describe a v7→v8 schema bump, `pages.kind`, unchecked tasks, and broader future-slice claims. The branch copies were already updated to the shipped reality: schema v8 was the landed baseline before the remaining slices, all 70 tasks are complete, and the narrower conversation-routing / fixed lease-expiry truths are explicitly documented.

## Applied rule

1. Resolve the six add/add conflicts to the artifact text that matches the shipped implementation, not the first version that reached `main`.
2. Preserve completed checkbox history and truth notes that explain the landed baseline and narrowed seams.
3. Treat the merge as documentation-truth repair only; no unrelated code or `.squad/` churn enters the commit.
