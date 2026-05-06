# mom history

## Active Sessions: 2026-04-30

**Status:** Post-merge coordination. Release v0.15.0 owned by Zapp. Coverage and review lanes complete.

## Recent Work (2026-04-28+)

- **2026-04-29T13:29:11Z:** Batch 3 review close — Professor rejected incomplete task closure, Nibbler rejected on safety. Mom reassigned to fix blockers.
- **2026-04-29T13:57:48Z:** Memory Cycle: Batch 3 Validation Gate FAIL — Windows lane 90.52% line coverage. Revision cycle running.
- **2026-04-29:** Batch 4 session work completed. Session type fix + task 12.7 reopen. 834 lib tests pass.
- **2026-04-28:** Batch 1 coverage lanes complete. Global line coverage 88.76% (honest ceiling without broader proof clusters).

## Key Learnings

- v7 bootstrap crash window requires empty `quaid_config` paired with fresh schema state (default collection only, no user rows in mutable tables).
- Bulk vault rewrites need root-scoped offline lease across same-root aliases to prevent mid-rewrite serve ownership claims.
- DEFAULT column is schema contract: discriminator early when same table stores two conceptually different row kinds.
- Error string contracts are bidirectional: all producers and consumers must agree. Enumerate all expectations before choosing format.

## Learnings

- [2026-05-04T07:22:12.881+08:00] On rename-before-commit write paths, typed semantic refusals need a preflight gate before sentinel/tempfile/rename, but the in-transaction check must still stay in place as the race backstop. Honest proof is blocked-state evidence: no new vault file, no new active raw bytes, no recovery escalation.
- [2026-05-04T07:22:12.881+08:00] When a leased queue row can be re-claimed, `job_id` stops being an ownership proof. Bind `done`/`failed` transitions to the dequeue generation already carried by the row (here: `attempts`) or a stale worker can close a newer lease after expiry.
- [2026-05-04T07:22:12.881+08:00] Watcher-driven archive-on-edit stays linear if the old head becomes the archive row and any existing predecessor is rewired onto that archive before the live head is updated. Whitespace-only saves need two proofs together: the handler must refuse to churn page/raw/file-state rows, and diff/full-hash classification must also suppress the same path so the no-op stays quiet on the next reconcile.
- [2026-05-04T07:22:12.881+08:00] A rename-only extracted whitespace no-op still needs a tracked-path handoff. If the early return deletes the old `file_state` row without moving it to the new path, the page becomes untracked even though page/raw state stayed unchanged.

---

## Archived Sessions

Earlier sessions (2026-04-27 and prior) archived to history-archive.md. Previous work includes:
- Batch 1 edge-case implementation (6.8 + cleanup)
- macOS preflight diagnostics (Issue #79/#80)
- Restore artifact reconciliation (Fry lockout fix)
- Vault Sync CI fix (6 failing tests, global registry state leak, frontmatter mismatch, OCC error format)
---

## Spawn Session — 2026-05-06T13:44:12Z

**Agent:** Scribe
**Event:** Manifest execution

- Decision inbox merged: 63 files
- Decisions archived: 1 entry (2026-04-29)
- Team synchronized