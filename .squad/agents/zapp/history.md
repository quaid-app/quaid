# zapp history

- [2026-04-29T07-04-07Z] History summarized and archived

## Learnings

- [2026-04-29T21:29:11.071+08:00] Pretag release-prep audits for Quaid need four truth checks together: `Cargo.toml` version, `.github/workflows/release.yml` tag gate, public install docs (`README.md`, `docs/getting-started.md`, website install tutorial), and `docs/roadmap.md` deferred-state wording. They can drift independently even when the branch is internally approved.
- [2026-04-29T21:29:11.071+08:00] Batch-style release docs drift in two directions at once: top-level release messaging can lag on the target version, while roadmap/deferred tables can still describe already-shipped admin commands as future work. The truth pass has to close both gaps in one commit before a release branch is really tag-ready.
- [2026-04-29T21:29:11.071+08:00] In a release worktree, never assume the checked-out HEAD is the tag target. Compare the intended SHA to `origin/main` and diff the release-critical files (`Cargo.toml`, release workflow, asset manifest, and public install docs) before tagging the exact commit.

## 20260429T173541Z — Team sync

**Scribe update:** Decisions merged (inbox → decisions.md), orchestration logs written, Batch 3 merge lane BLOCKED by codecov/patch.


## 2026-04-29 Release Checkpoint
- Zapp: release prep COMPLETE (v0.12.0 validation green)
- Amy: docs truth review STARTED
- Leela: merge lane STARTED
- Scribe: memory checkpoint logged
