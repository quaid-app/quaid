## Context

`p3-polish-benchmarks` started life as a catch-all “finish everything left in Phase 3” proposal:
skills finalization, benchmark harnesses, release tooling, and public polish. That scope is too wide for the work that is actually ready to land now.

The current repo state shows a narrower, reviewable problem:
- `README.md` and docs-site copy still contains stale status/install language.
- `.github/workflows/ci.yml` has no coverage job even though coverage visibility is now a release-readiness ask.
- `.github/workflows/docs.yml` deploys the site from `website/`, but the public docs/release story is not yet organized around current status, install paths, and contribution entry points.
- `.github/workflows/release.yml` already builds assets and checksums, but the public-facing contract around names, docs, and review sign-off is not explicit enough.

This change therefore focuses on the release-facing surface the team can complete now: honest docs, free coverage on `main`, docs-site improvements, and GitHub Release polish. Broader benchmark and “new distribution channel” work remains real, but is explicitly deferred.

## Goals / Non-Goals

**Goals:**
- Define a smaller Phase 3 slice that is ready for implementation now.
- Make README, docs-site, and release workflow tell the same story about current product status and supported install paths.
- Add a free, visible coverage path for pushes to `main` and PRs targeting `main`.
- Give Fry, Amy, Hermes, and Zapp clear ownership boundaries and review hand-offs.
- Keep npm global distribution and “simple installer” UX visible as future work without pretending they already ship.

**Non-Goals:**
- Implement the remaining Phase 3 skills (`briefing`, `alerts`, `research`, `upgrade`, `enrich`) in this change.
- Build the full benchmark program (BEIR regressions, LongMemEval, LoCoMo, Ragas) in this change.
- Add a new package registry distribution channel (`npm`, Homebrew, winget, curl installer, etc.) in this change.
- Redefine runtime product scope or change core CLI/MCP contracts.

## Decisions

### 1. Split “ready now” release work from later benchmark/distribution work

**Decision:** `p3-polish-benchmarks` becomes the release/docs/coverage/docs-site slice and is no longer the umbrella for every remaining Phase 3 idea.

**Rationale:** The current blocker is not “benchmarks are unfinished”; it is that public release posture is fragmented. Narrowing the change makes `openspec apply` usable and keeps implementation reviewable.

**Alternative considered:** Keep the original benchmark-heavy proposal and just append docs tasks. Rejected because the change would remain too large and would continue to mix ready-now work with not-yet-ready follow-ons.

### 2. Lower the dependency from Phase 2 to Phase 1-complete release surface work

**Decision:** This change depends on `p1-core-storage-cli`, not `p2-intelligence-layer`.

**Rationale:** Coverage visibility, README honesty, docs-site polish, and GitHub Release hardening do not require all Phase 2 capability work to finish first. They are public-surface improvements that can be prepared as soon as the core CLI/release surface exists.

**Alternative considered:** Leave `depends_on: p2-intelligence-layer`. Rejected because it blocks ready documentation/workflow work behind unrelated feature scope.

### 3. Use a free coverage path centered on GitHub Actions outputs

**Decision:** The coverage implementation should generate coverage on GitHub Actions for pushes to `main` and PRs to `main`, publish machine-readable output plus a human-readable surface through GitHub-native artifacts/job summaries, and treat any optional third-party upload as non-blocking.

**Rationale:** The requirement is “free coverage on push to main,” not “buy or depend on a paid dashboard.” GitHub-hosted outputs guarantee the repo always has a visible coverage surface even if an external service is unavailable.

**Alternative considered:** Third-party-only coverage (for example, a hosted dashboard with no GitHub artifact fallback). Rejected because it creates an avoidable availability and policy dependency.

### 4. Make public docs explicitly distinguish “supported now” vs “planned later”

**Decision:** README and docs-site content must present a three-way contract:
1. supported now,
2. in-progress / planned surface,
3. explicitly deferred follow-on work.

**Rationale:** The current drift comes from mixing aspirational copy with current status. Separating supported-now from planned-later keeps docs useful without understating the roadmap.

**Alternative considered:** Rewrite docs to only describe current implemented behavior. Rejected because the repo still uses spec-driven planning and needs public roadmap context.

### 5. GitHub Releases remain the only supported binary channel in this change

**Decision:** This change will only harden GitHub Releases + build-from-source guidance. npm global installation and a simplified installer remain documentation-only follow-ons.

**Rationale:** The repo already has a release workflow and checksum model. New installers and registries multiply operational surface area, support burden, and trust-chain questions before the core release contract is fully polished.

**Alternative considered:** Fold npm packaging or a curl installer into this same change. Rejected as premature scope growth.

### 6. Ownership is split by surface, not by file type alone

**Decision:** Task routing is by domain:
- **Fry** owns CI/release workflow implementation.
- **Amy** owns stale-doc remediation and README/docs wording.
- **Hermes** owns docs-site UX, navigation, and deployment flow.
- **Zapp** owns public release copy, launch checklist, and deferred-channel messaging.

**Rationale:** This matches existing team specialties and avoids making Fry the default owner for all public-surface work.

## Risks / Trade-offs

- **[Coverage tool choice introduces CI churn]** → Mitigation: keep the spec on outputs and free availability; Fry can choose the exact Rust coverage tool as long as it satisfies the contract and runs in existing GitHub Actions.
- **[Docs can drift again if README and website evolve separately]** → Mitigation: define a single public status/install matrix and require both surfaces to match it.
- **[Docs workflow may deploy stale messaging if it only watches `website/**`]** → Mitigation: Hermes should either expand triggers/build inputs or document the source-of-truth sync path explicitly.
- **[Release/docs polish may be mistaken for “all release work is done”]** → Mitigation: keep benchmark work and npm/installer work called out as explicit non-goals/follow-ons in docs and release notes.
- **[Scope narrowing could leave orphaned benchmark work]** → Mitigation: record the deferral in a decision note and in the proposal/design non-goals so later planning can pick it up cleanly.

## Migration Plan

1. Update the OpenSpec artifacts first so the scope reset is explicit before implementation starts.
2. Fry lands workflow changes for coverage/release hardening behind the new specs.
3. Amy and Hermes land README/docs-site changes against the same status/install matrix.
4. Zapp performs release-surface review: asset names, install instructions, deferred channel wording, and launch note shape.
5. Any remaining benchmark/npm/installer work is proposed separately after this slice lands.

Rollback is straightforward because this change only affects docs and GitHub workflows: revert the workflow/docs commits and the repo returns to the previous public posture. No database or runtime migration is involved.

## Open Questions

1. Should the stable public coverage URL be a GitHub Pages HTML report, an Actions artifact link pattern, or both?
2. Does the docs site need to rebuild when `README.md` changes, or should mirrored website content remain the sole published source?
3. Should coverage be informational only in this slice, or is there already enough signal to introduce a fail-under gate later?
