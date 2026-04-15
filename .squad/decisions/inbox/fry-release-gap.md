# Decision: Phase 1 Release Gap — v0.1.0 Tag Never Pushed

**Date:** 2026-04-15  
**Author:** Fry  
**Status:** Proposed

## Context

Phase 1 (Core Storage, CLI, Search, MCP) is fully complete:
- All 34 tasks (T01–T34) are `[x]` in the archived tasks.md
- All 9 ship gates (SG-1 through SG-9) are `[x]`
- PR #12 merged Phase 1 to main
- PR #15 merged release-readiness work to main
- CI passes on main (conclusion: success)
- `Cargo.toml` already has `version = "0.1.0"`
- The release workflow (`.github/workflows/release.yml`) triggers on `v*.*.*` tag pushes

But **no v0.1.0 tag was ever pushed**, so the release workflow never fired, and no GitHub Release exists.

Meanwhile, all public docs (README, getting-started, roadmap, website) still said "Phase 1 in progress" and "not yet available", which is now inaccurate.

## Decision

1. **Update all docs** to reflect Phase 1 as complete (README, docs/, website/).
2. **After this PR merges**, push the `v0.1.0` tag on main to trigger the release workflow:
   ```bash
   git tag v0.1.0 && git push origin v0.1.0
   ```
3. **Verify** the release against `.github/RELEASE_CHECKLIST.md` once the workflow completes.

## Rationale

The roadmap commits to releasing v0.1.0 after Phase 1. Phase 1 is done. The gap is purely operational — nobody pushed the tag. The docs over-promised "in progress" status when the work was already shipped.
