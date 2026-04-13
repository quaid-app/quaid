---
updated_at: 2026-04-13T00:00:00Z
focus_area: Sprint 0 — revision cycle (addressing Nibbler rejections)
active_issues: []
---

# What We're Focused On

**Sprint 0 revision in progress.** Nibbler rejected multiple Sprint 0 artifacts. Fry owns this revision cycle (Leela is locked out for this round).

**Revisions applied:**
1. ✅ `Cargo.toml`: added `env` feature to `clap` so `#[arg(env)]` compiles
2. ✅ `src/main.rs`: replaced `~/brain.db` default with platform-safe home-dir resolution
3. ✅ `.github/workflows/ci.yml`: removed musl release build + static-link verification (out of scope for Sprint 0 per proposal)
4. ✅ `.github/workflows/release.yml`: fixed tag trigger to valid glob syntax; pinned `cross` to v0.2.5 instead of git HEAD
5. ✅ `openspec/changes/sprint-0-repo-scaffold/proposal.md`: aligned scope/non-goals with actual CI behavior
6. ✅ `openspec/changes/p1-core-storage-cli/proposal.md`: added explicit OCC semantics and conflict behavior for all write paths
7. ✅ `src/schema.sql`: `knowledge_gaps` now stores `query_hash` by default; raw `query_text` is nullable and only retained after explicit approval

**Next steps:**
1. Nibbler re-review of revised artifacts
2. Commit scaffold on branch `sprint-0/scaffold` and open PR to main
3. Create GitHub issues for Phase 1 workstreams
4. Fan out to Fry for Phase 1 implementation after Sprint 0 merges

**Phase sequence:**
- Sprint 0 → Phase 1 (Core) → Phase 2 (Intelligence) → Phase 3 (Polish + Release)
- Each phase has a hard gate. No skipping.
