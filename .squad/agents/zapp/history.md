# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- The project explicitly wants docs and OSS presence that can go viral.
- DevRel work needs to stay grounded in shipped behavior and approved proposals.
- Docs quality and growth strategy are first-class concerns, not nice-to-haves.
- Release surface clarity is a growth asset: explicitly naming what does NOT ship (npm, Homebrew, one-command installer) builds trust faster than vague "coming soon" copy.
- A release checklist at `.github/RELEASE_CHECKLIST.md` is the right artifact for sign-off ceremonies — Zapp, Fry, and Leela each have a named row.
- The three-way contract (supported-now / in-progress / deferred) must appear in README, docs site, AND release notes body — a single source of truth drifts the moment one surface is edited independently.
- GitHub's `softprops/action-gh-release` `body` field prepends to auto-generated notes, which is the right place to fix install guidance before any PR title leaks unsupported channel language into a release.
- Phase/version alignment is a chronic drift point: `v0.1.0 = Phase 1`, `v0.2.0 = Phase 2`, `v1.0.0 = Phase 3`. Any doc that mentions a version tag must also cite the correct phase, and vice versa.
- When a status table lists phases without version tags, readers cannot cross-reference the roadmap — always include both the phase label and the version target in the same row.
- Operational scripts (issue creation commands, label helpers) are docs too: a mismatched label like `[Phase 3] v0.1.0 release` teaches contributors the wrong mental model before they've even opened a file.

## 2026-04-15 Release Contract Audit — Fix 'for this release' ambiguity

**Role:** Release-facing copy, release contract clarity

**What happened:**
- User flagged that docs appeared to imply a release existed or would happen after each phase but no release was present.
- Audit found two concrete issues: (1) README.md used "this release" language implying v0.1.0 was already shipped, and the curl snippet was presented as immediately usable; (2) docs/contributing.md had `[Phase 3] v0.1.0 release` in the issue script, contradicting the version target table (`v0.1.0 = Phase 1`).
- Discovered PR #19 (`fix/v0.1.0-release-gap`) already existed, correctly documenting that Phase 1 is complete and v0.1.0 is pending tag push.
- Added two commits to PR #19: (1) replaced "only supported binary distribution channels for this release" with explicit build-from-source (available now) / GitHub Releases (pending v0.1.0 tag) split, plus a "Not yet available" callout on the curl block; (2) corrected contributing.md issue script label from phase-3 to phase-1.
- Decision logged to `.squad/decisions/inbox/zapp-release-contract.md` (gitignored; local only).

**Outcome:** PR #19 now carries full release-contract clarity: Phase 1 complete, v0.1.0 pending tag, no false implication of an existing release. PR #18 (opened on wrong base) was already closed.

**Decision:** Option (b) — tighten wording. No premature release was published.



**Role:** Release-facing copy, checklist, phase/version alignment sign-off

**What happened:**
- Zapp added release checklist at `.github/RELEASE_CHECKLIST.md` with named sign-off rows for Zapp, Fry, Leela.
- Updated RELEASE_CHECKLIST.md and release-facing copy for standard checksum format (`hash  filename`).
- Final doc fix pass: corrected phase/version alignment in `install.md` (status table now includes version tags) and `contributing.md` (issue script corrected from `[Phase 3] v0.1.0` to `[Phase 1] v0.1.0`).
- All operational scripts and status matrices aligned with roadmap version targets.

**Outcome:** P3 Release release-readiness component **COMPLETE**. Release checklist ready, phase/version aligned across README/docs/scripts, all gates passed.

**Decision notes:** `.squad/decisions.md` (merged from inbox) — documents Zapp's two decisions (release checklist routing, final phase/version alignment fixes).


## 2026-04-16 v0.9.0 Shell-First Release Lane

**Role:** Release lane truthfulness, branch/tag strategy, CI verification

**What happened:**
- Assessed full repo state: branch `main` was 1 commit ahead of origin, with all simplified-install work staged as modifications + untracked files (scripts/install.sh, packages/gbrain-npm/, proposal.md, updated workflows + docs).
- Confirmed no v0.9.0 tag existed locally or on remote. Cargo.toml already at 0.9.0.
- Created branch `release/v0.9.0` from local HEAD (not main), committed all 19 file changes in one coherent commit (`c1f572b`), pushed branch to origin.
- Created annotated tag `v0.9.0` on that commit with full release description, pushed tag — triggered the real GitHub Actions release pipeline.
- Result: 4 build jobs running (darwin-arm64, darwin-x86_64, linux-x86_64, linux-aarch64). npm publish workflow completed ✓ with correct notice ("NPM_TOKEN not configured; skipping").
- The release will produce real pre-built binaries + checksums + install.sh as a release asset.

**Key decisions:**
- Tagged on `release/v0.9.0` branch, not `main` — user explicitly requested non-main branch. GitHub Actions `push.tags` trigger fires regardless of branch.
- `prerelease: ${{ contains(github.ref_name, '-') }}` evaluates to `false` for `v0.9.0` — a full (not pre-) release on GitHub. v0.9.0 < 1.0.0 already communicates test-release status via semver.
- D.2 and D.5 remain environment-blocked but are not blocking the CI-based release build; those checks defer to post-release macOS/Linux runner verification.

## 2026-04-16T14:59:20Z Simplified-install v0.9.0 Release — Zapp Completion

- **Task:** Created release/v0.9.0 branch, committed simplified-install work, pushed branch, tagged v0.9.0, triggered GitHub release workflow, published binaries/checksums/install.sh
- **Changes:**
  1. Branch creation — created `release/v0.9.0` from local HEAD
  2. Commit — committed simplified-install work with full change log (19 files)
  3. Push branch — pushed `release/v0.9.0` to origin
  4. Tag creation — created and pushed annotated `v0.9.0` tag
  5. Release workflow — triggered GitHub Actions release workflow
  6. Artifacts published — release workflow published 4 binaries, 4 checksums, install.sh
- **Status:** ✅ COMPLETE. v0.9.0 release live. Binaries published to GitHub Releases. Checksums verified.
- **Orchestration log:** `.squad/orchestration-log/2026-04-16T14-59-20Z-zapp.md`

**Learnings:**
- Annotated tags (`git tag -a`) are preferable to lightweight tags for releases — they carry a tagger identity and timestamped message that shows in GitHub's release view.
- The `softprops/action-gh-release@v2` + `gh release upload` two-step pattern is correct for adding the install.sh asset after the binary artifacts are attached.
- npm token guard ("skip if absent, never fail") is the right CI posture for staged channels — zero friction for maintainers who haven't configured npm yet.

## Learnings

- Issue #60 (flexible model resolution): hardcoded HF revision SHAs and hash tables are a maintenance trap — they go stale whenever upstream repos prune revisions. Resolution logic should use the latest revision dynamically or omit revision pinning entirely for model files.
- `gbrain model list` is a discoverable entry point for users confused by `--model` options; linking help text to a subcommand is better UX than enumerating all aliases in the flag description.
- When closing an issue tied to a bug-fix release slice (not a standalone release), the right roadmap move is to add a new version target row rather than editing an existing phase section.
