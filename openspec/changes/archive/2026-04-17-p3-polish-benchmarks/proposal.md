---
id: p3-polish-benchmarks
title: "Phase 3: Release Readiness, Coverage, and Docs Polish"
status: shipped
type: feature
phase: 3
owner: leela
reviewers: [fry, amy, hermes, zapp, kif, scruffy]
created: 2026-04-13
archived: 2026-04-17
depends_on: p1-core-storage-cli
---

# Phase 3: Release Readiness, Coverage, and Docs Polish

## Why

The repo has outgrown its current public story. `README.md`, the docs site, and the release/coverage workflows no longer line up cleanly with what the tree actually supports, which makes release prep noisy and reviewable only by tribal knowledge.

The existing `p3-polish-benchmarks` proposal is also too broad for the work that is ready right now. This change narrows the immediate slice to release readiness, docs honesty, free coverage visibility on `main`, and docs-site polish, while explicitly deferring npm global distribution and one-command installer UX until they are implementation-ready.

## What Changes

- Tighten the Phase 3 scope to public release readiness: GitHub release assets, checksum/documentation alignment, and a reviewable ship surface.
- Add free coverage reporting on pushes to `main` and PRs targeting `main`, with coverage output visible through GitHub-hosted or other no-cost public surfaces.
- Fix stale public docs in `README.md` and the docs site so current status, supported install paths, and deferred work are stated honestly.
- Improve the docs site build/deploy flow and information architecture around install, release, coverage, and contribution entry points.
- Document npm global distribution and simplified installer UX as follow-on work, not as part of this implementation slice.

## Capabilities

### New Capabilities
- `release-readiness`: GitHub release workflow hardening, checksum/install alignment, and a reviewable public release checklist.
- `coverage-reporting`: Free coverage generation and visibility on pushes to `main` and PRs to `main`.
- `documentation-accuracy`: Honest, synchronized README and public docs for current status, supported install paths, and deferred work.
- `docs-site`: Docs-site build/deploy and navigation improvements for release, install, and contribution flows.

### Modified Capabilities
- None.

## Impact

- `README.md` public install/status/release copy
- `website/**` content, navigation, and GitHub Pages deployment behavior
- `.github/workflows/ci.yml`, `docs.yml`, and `release.yml`
- Release-facing asset names, checksum expectations, and coverage/report links
- Follow-on planning for npm packaging and installer UX, without adding those delivery channels yet
