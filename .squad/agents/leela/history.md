# leela history

- [2026-04-30T13:53:46Z] COMPLETED: Merged PR #131 to main at commit 6d36f6f0cb6fd4afb2133f23ceaf018079b1a50e after fixing Linux CLI truth failures, resolving review threads, and revalidating fmt/clippy/tests
- [2026-04-29T07-04-07Z] History summarized and archived

## Learnings

- 2026-05-02T18:31:04.840+08:00 — **Retroactive OpenSpec pattern:** When a change ships without a prior proposal, create the change with `openspec new change <name>`, write all artifacts to reflect shipped reality (not future plans), and pre-check all tasks in `tasks.md` so `openspec status` reports `isComplete: true`. Add `**Status:** complete — shipped in vX.Y.Z (PR #NNN)` to proposal.md and include a bundled-fixes section in tasks.md for any follow-up commits in the same PR.
- 2026-05-02T18:31:04.840+08:00 — **Retroactive change name convention:** Derive the kebab-case change name from the feature branch slug used in the implementing PR (e.g., `feat/namespace-isolation-137` → `namespace-isolation`).
- 2026-05-02T18:31:04.840+08:00 — **namespace-isolation (v0.16.0) key files:** `src/core/namespace.rs`, `src/core/db.rs` (legacy rebuild), `src/core/search.rs`/`fts.rs`/`progressive.rs` (namespace-aware variants), `src/commands/namespace.rs`, `src/mcp/server.rs` (2 new tools + 4 updated), `tests/namespace_isolation.rs`. Schema v6 adds `namespace TEXT NOT NULL DEFAULT ''` to `pages`, new `namespaces` table, updated UNIQUE constraint to `(collection_id, namespace, slug)`.

- 2026-04-30T06:37:20.531+08:00 — When the working tree is parked on a stale release branch, read tasks.md from `origin/main` (via `git show origin/main:...`) rather than from the local checkout — the local view will lag by the entire merged batch and create false "all open" readings.
- 2026-04-30T06:37:20.531+08:00 — Batch 4 target scope (v0.13.0): `12.1` (complete 13-step rename-before-commit audit), `12.6` (expected_version contract), `12.6a` (CLI write routing single-file), `12.7` (tests). Task `12.6b` was closed in Batch 3 — only verify presence, do not re-implement.
- 2026-04-30T06:37:20.531+08:00 — The correct Batch 4 worktree setup is: `git worktree add ..\quaid-vault-sync-batch4-v0130 -b spec/vault-sync-engine-batch4-v0130 origin/main` from `D:\repos\quaid`. Starting SHA `5a8bdf0` (v0.12.0). Never branch Batch 4 from a stale release branch.
- 2026-04-30T06:37:20.531+08:00 — `now.md` is stale (last updated 2026-04-25, references `spec/vault-sync-engine` as active branch). Scribe should refresh it after Batch 4 lands; it does not block execution.
- [2026-04-30T06:37:20Z] Batch 4 branch routing decision merged to team ledger. Worktree setup required before Fry begins implementation.
- 2026-04-30T12:07:19.084+08:00 — `D:\repos\quaid` is still parked on `release/v0.11.0`, sits 31 commits behind `origin/main` (`9bdb34b`, tag `v0.13.0`), and its local `tasks.md` still shows Batch 3 and Batch 4 closures as open. For Batch 5 gating, treat `origin/main` as the only task-truth source.
- 2026-04-30T12:07:19.084+08:00 — Safe Batch 5 start lane is a fresh sibling worktree `D:\repos\quaid-vault-sync-batch5-v0140` on branch `spec/vault-sync-engine-batch5-v0140` from `origin/main`; remaining scope is `11.9`, `12.6c`–`12.6g`, and `17.5ii10`–`17.5ii12`, and release `v0.14.0` stays blocked until that batch is reviewed, merged to `main`, and coverage is revalidated above 90%.
- 2026-04-30T19:26:58.386+08:00 — After PR `#126` merged, `origin/main` advanced to merge commit `05b8c331765f23dc65c67ca3167b7dc38256a328`; release prep must start from that exact SHA, not from the old Batch 5 implementation worktree.
- 2026-04-30T19:26:58.386+08:00 — The safe `v0.14.0` release-prep lane is the clean sibling worktree `D:\repos\quaid-v0.14.0-release` on branch `release/v0.14.0`; the existing `D:\repos\quaid-vault-sync-batch5-v0140` branch is ahead locally and is not a trustworthy release base.
 
 
- 2026-04-29T20:33:01.970+08:00 — Safe Batch 3 implementation lane is the clean sibling worktree `D:\repos\quaid-vault-sync-batch3-v0120` on branch `spec/vault-sync-engine-batch3-v0120`, created from `origin/main` at `fdc20a0`. Keep the dirty `D:\repos\quaid` checkout on `release/v0.11.0` untouched.
- 2026-04-29T20:33:01.970+08:00 — Batch 3 source-of-truth artifacts are `openspec\changes\vault-sync-engine\implementation_plan.md` (Batch 3 section) and `openspec\changes\vault-sync-engine\tasks.md` items `5a.5`, `5a.5a`, `9.2a`, `5a.7`, `17.5ww`, `17.5ww2`, `17.5ww3`, `17.5ii9`.
- 2026-04-29T20:33:01.970+08:00 — Release prep for `v0.12.0` must start from merged `main` (current main is `0.11.6`), follow `.github\workflows\release.yml` + `.github\RELEASE_CHECKLIST.md`, and manually verify the coverage report stays above 90% because CI publishes coverage evidence but does not enforce the threshold itself.
- 2026-04-29T21:29:11.071+08:00 — Batch 3 ancestry is confirmed clean: branch `spec/vault-sync-engine-batch3-v0120` was created from `origin/main` at `fdc20a0`, not from `origin/release/v0.11.0`, so branch-base conflict recovery is not required.
- 2026-04-29T21:29:11.071+08:00 — Merge-lane rule: even after `CI/Check`, `Test`, `Coverage`, offline benchmarks, and macOS preflight jobs go green, the PR can still stay policy-blocked by a failing third-party status like `codecov/patch`; do not admin-merge around it.
- 2026-04-29T21:29:11.071+08:00 — `release/v0.12.0` only cleared the final merge lane after the blockers were fixed in-branch: serialize the process-global env-var tests, cover the env-guard restore path so `codecov/patch` passes, address the live docs review thread, resolve review conversations, and then merge PR `#123` cleanly to `main` at `5a8bdf068bf54be52f9b2bc661af34056473221a`.

## 20260429T173541Z — Team sync

**Scribe update:** Decisions merged (inbox → decisions.md), orchestration logs written, Batch 3 merge lane BLOCKED by codecov/patch.

## 20260429T132911Z — Session: Merge Lane Active

**Status:** STARTED
**PR:** #122 (spec/vault-sync-engine-batch3-v0120)
**Policy Gate:** codecov/patch pending rerun

**Summary:** Watching PR #122 checks and merging as soon as GitHub policy allows.

**Details:**
- All required checks green or pending
- Policy blocked on codecov/patch status
- No admin merge; waiting for green
- Ready to merge on gate clearance


## 2026-04-29 Release Checkpoint
- Zapp: release prep COMPLETE (v0.12.0 validation green)
- Amy: docs truth review STARTED
- Leela: merge lane STARTED
- Scribe: memory checkpoint logged

## 2026-04-29T13:29:11Z Session Outcome
- Status: COMPLETE (merge lane closed)
- PR #123: release/v0.12.0 merged to main
- Release SHA: 5a8bdf068bf54be52f9b2bc661af34056473221a
- All quality gates cleared: test race fixed, codecov/patch cleared, review threads resolved
- Next: Awaiting v0.12.0 tag creation
- 2026-04-30T06:37:20.531+08:00 — **Batch 4 scope reconciliation:** When `tasks.md` and `implementation_plan.md` describe the same task differently, the implementation_plan is authoritative for batch scope. Fix `tasks.md` to match; never expand scope in `tasks.md` beyond what the plan intends for the batch.
- 2026-04-30T06:37:20.531+08:00 — **IPC proxy deferral pattern:** A security-critical subsystem (IPC socket with kernel peer-UID verification) must land and be reviewed as its own batch before any client code is built against it. Building client-side proxy before the server socket design is locked creates a dependency inversion and risks shipping unauthenticated writes.
- 2026-04-30T06:37:20.531+08:00 — **Scope note format in tasks.md:** When narrowing a task from its original spec description, add a `> **Scope note (Author, date):**` annotation inline below the task line explaining the narrowing rationale and which batch/tasks will complete the original scope. This creates an auditable trail without losing original intent.
- 2026-04-30T06:37:20.531+08:00 — Batch 4 target scope (v0.13.0): `12.1` (complete 13-step rename-before-commit), `12.6` (expected_version contract), `12.6a` (refuse-when-live stub, NOT proxy), `12.6b` (verify only — already closed in Batch 3), `12.7` (tests). IPC tasks (11.9, 12.6c–g) are Batch 5.
