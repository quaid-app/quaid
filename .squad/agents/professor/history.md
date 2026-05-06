# professor history

- [2026-04-29T07-04-07Z] History summarized and archived

## Learnings

- [2026-05-04T07:22:12.881+08:00] Re-review lesson: a truth-repair is sufficient when it rewrites every checked artifact to the exact shipped contract and resets the task boundary to the already-landed baseline. Once proposal, design, tasks, and specs all name the real schema surface consistently, prior contract-truth rejection should clear without inventing new blockers.
- [2026-05-04T07:22:12.881+08:00] Conversation-memory schema review: a green task box is not honest if proposal/tasks still describe the wrong contract. Decision inbox notes can justify a narrower implementation, but the OpenSpec artifacts themselves must be rewritten to match shipped names/guards before review can approve.
- [2026-04-29T20:33:01.970+08:00] Batch 3 vault-sync review: task truth must match operator-facing behavior exactly; if OpenSpec says a live-owner refusal must name pid/host and tell the operator to stop serve first, tests must assert that guidance, not just the error tag.
- [2026-04-29T21:29:11.071+08:00] Batch 3 re-review: bulk vault rewrites only honestly close when the guard is root-scoped twice — refuse same-root live owners before alias insertion, then hold an offline owner lease across every same-root row for the full batch. Operator guidance is part of the contract; if the spec says "stop serve first," the CLI text must say it.
- [2026-04-29T21:29:11.071+08:00] Coverage-lift follow-ups are acceptable when they stay in same-file helper tests and explicitly narrow remaining platform proof. Repo-wide Windows coverage is not evidence to relabel Unix-only vault-sync guarantees as Windows-certified.
- [2026-04-30T12:07:19.084+08:00] Batch 5 IPC review: local proxy auth is only trustworthy when kernel UID/PID checks lead and protocol/session/path checks merely cross-check; any live-owner fallback to direct CLI vault writes reopens the dedup race the socket exists to close.
## 2026-04-29T13:29:11Z — Batch 3 review close

- **Professor:** Rejected Batch 3 on incomplete task closure (`12.6b`/`17.5ii9`). Error text lacks "stop serve first" guidance. Tests incomplete.
- **Nibbler:** Rejected Batch 3 on safety: live-owner guard keyed to `collection_id` (not unique), bulk rewrite lacks offline lease, test coverage insufficient.
- **Mom:** Reassigned to fix both blocking findings. Fry locked out.
- **Scruffy:** Paused validation; coverage lane held pending implementation revisions.


## 2026-04-29 Session: Batch 3 Multi-Agent Review

**Session ID:** 2026-04-29T13:29:11.071Z
**Agents:** Professor, Nibbler, Scruffy
**Outcome:** Professor APPROVE, Nibbler APPROVE, Scruffy FAIL/STARTED

### Session Summary

Batch 3 revision received multi-agent re-review for same-root alias race and offline-lease closure. Professor and Nibbler approved the fix. Scruffy's revalidation failed on Windows coverage gap; coverage fix work started. Task claims narrowed to match actual proof seams.

### Cross-Agent Dependencies

- Professor → Nibbler: Review results aligned (same-root alias closed + root-scoped lease works)
- Nibbler → Scruffy: Revalidation launched on Windows lane; coverage still insufficient
- Scruffy: Blocking on honest Windows coverage for 17.5ww/ww2/ww3 or narrowed claims


## 2026-04-29 — Delta review started
- Task: Review 67f4091..397d7c7 (Scruffy's coverage fix)
- Scope: Maintainability + truth
- Status: In progress

## 2026-04-30T00:30:31Z
- **Action:** Reviewed team progress and confirmed release readiness
- **Status:** APPROVED

- [2026-04-30T06:37:20.531+08:00] Reviewed Batch 4 checkpoint on `spec/vault-sync-engine-batch4-v0130`; kept 12.1/12.6/12.6a closed, reopened 12.7, and rejected the gate because `insert_write_dedup()` still silently accepts duplicate inserts.
- [2026-04-30T08:30:31.626+08:00] Re-reviewed Batch 4 on `spec/vault-sync-engine-batch4-v0130`; approved the revised partial Batch 4 checkpoint, confirmed the `session_type='serve'` live-owner fix, and kept 12.7 open with the honest non-observable duplicate-dedup note.
- [2026-04-30T08:30:31.626+08:00] Final review: APPROVED commit `714ec48` as the remaining restore/remap handshake typing fix; Batch 4 is now an approved partial checkpoint with task `12.7` still intentionally open.
- [2026-04-30T08:30:31.626+08:00] Reviewed Fry's 12.7 closure attempt on `spec/vault-sync-engine-batch4-v0130`; approved final Batch 4 closure after verifying duplicate dedup inserts now fail closed with typed error coverage and passing `cargo check --all-targets --quiet` plus `cargo test --quiet -j 1`.
## Batch: Orchestration Consolidation
**Timestamp:** 2026-05-04T00:00:30Z

- Decisions consolidated: inbox merged → decisions.md (8 files)
- Archive: 5698 lines archived to decisions-archive.md
- Status: All agents' work reflected in team memory
---

## Spawn Session — 2026-05-06T13:44:12Z

**Agent:** Scribe
**Event:** Manifest execution

- Decision inbox merged: 63 files
- Decisions archived: 1 entry (2026-04-29)
- Team synchronized