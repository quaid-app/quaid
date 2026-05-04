# zapp history

- [2026-04-30T13:53:46Z] STARTED: v0.15.0 release lane ownership. Coordinator action initiated
- [2026-04-29T07-04-07Z] History summarized and archived

## Learnings

- [2026-04-29T21:29:11.071+08:00] Pretag release-prep audits for Quaid need four truth checks together: `Cargo.toml` version, `.github/workflows/release.yml` tag gate, public install docs (`README.md`, `docs/getting-started.md`, website install tutorial), and `docs/roadmap.md` deferred-state wording. They can drift independently even when the branch is internally approved.
- [2026-04-29T21:29:11.071+08:00] Batch-style release docs drift in two directions at once: top-level release messaging can lag on the target version, while roadmap/deferred tables can still describe already-shipped admin commands as future work. The truth pass has to close both gaps in one commit before a release branch is really tag-ready.
- [2026-04-29T21:29:11.071+08:00] In a release worktree, never assume the checked-out HEAD is the tag target. Compare the intended SHA to `origin/main` and diff the release-critical files (`Cargo.toml`, release workflow, asset manifest, and public install docs) before tagging the exact commit.
- [2026-05-04T07:22:12.881+08:00] For Quaid release lanes, green local gates are not enough for a truthful draft PR: the branch must be pushed and coherent, public version/tool-count copy must match the real landed scope, and moved-file links like `roadmap.md` / `MIGRATION.md` must either be repaired or preserved.
- [2026-05-04T07:22:12.881+08:00] When a branch inherits broader roadmap/spec ancestry than its actually landed implementation slice, a truthful draft PR has to name the exact pushed slice, call out the ancestry noise, and list explicit non-claims so unfinished capabilities are not read as shipped.
- [2026-05-04T07:22:12.881+08:00] When a draft PR picks up a new landed slice mid-flight, update the body immediately: name the exact pushed commit-backed surface, move the "remaining work starts at" boundary, and keep explicit non-claims so reviewers do not mistake proposal scope for shipped scope.
- [2026-05-04T07:22:12.881+08:00] GitHub `mergeable_state: dirty` is not just stale by default; confirm with a merge simulation. For conversation-memory work, the minimal coordinator action can be a narrow main refresh plus resolving spec-only add/add conflicts rather than any product rework.
- [2026-05-04T07:22:12.881+08:00] When a draft PR blocker is cleared by a follow-up fix, refresh the body to say the slice is now approved, name the specific follow-up seam it closed, and keep the larger change pinned to the next unfinished task boundary.
- [2026-05-04T07:22:12.881+08:00] When a draft PR crosses from an approved wave into the next in-flight wave, refresh three facts together: what is now approved/complete, what the next claimed surface is, and the freshly reproduced conflict list. Old conflict counts can drift even if the product scope claim does not.

## 20260429T173541Z — Team sync

**Scribe update:** Decisions merged (inbox → decisions.md), orchestration logs written, Batch 3 merge lane BLOCKED by codecov/patch.


## 2026-04-29 Release Checkpoint
- Zapp: release prep COMPLETE (v0.12.0 validation green)
- Amy: docs truth review STARTED
- Leela: merge lane STARTED
- Scribe: memory checkpoint logged

## 2026-04-30T00:30:31Z
- **Action:** Reviewed final coordination updates and approved team workflow
- **Status:** APPROVED
## Batch: Orchestration Consolidation
**Timestamp:** 2026-05-04T00:00:30Z

- Decisions consolidated: inbox merged → decisions.md (8 files)
- Archive: 5698 lines archived to decisions-archive.md
- Status: All agents' work reflected in team memory
