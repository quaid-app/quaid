# Phase 3 Execution Checklist — Release Readiness, Coverage, and Docs Polish

**Lead:** Leela  
**Implementers:** Fry (workflows), Amy (README/docs), Hermes (docs site), Zapp (release surface)  
**Reviewers:** Kif (release/coverage plan), Scruffy (coverage/docs verification), Zapp (launch wording)  

---

## 1. Fry — CI and release workflow groundwork

- [x] 1.1 Audit `.github/workflows/ci.yml`, `docs.yml`, and `release.yml` against the new specs and list the exact workflow deltas needed for coverage, docs validation, and release-surface consistency
- [x] 1.2 Add a coverage job that runs on pushes to `main` and PRs targeting `main`, reusing existing test execution instead of inventing a separate unreviewed test path
- [x] 1.3 Publish machine-readable and human-readable coverage outputs through free surfaces (artifact, job summary, Pages, or equivalent) and make any optional third-party upload non-blocking
- [x] 1.4 Harden `release.yml` so artifact names, checksum files, and verification steps match the public install contract exactly

## 2. Amy — stale-doc cleanup and honest install/status copy

- [x] 2.1 Audit `README.md` and public docs for stale status language (`not started`, `planned API`, outdated release claims, unsupported install commands)
- [x] 2.2 Rewrite README status/install/release sections so they separate supported-now channels from planned-later npm or installer work
- [x] 2.3 Update docs pages that currently present speculative or stale copy so they match the same status/install matrix as README
- [x] 2.4 Add explicit “deferred follow-on” wording for npm global distribution and simple installer UX without adding implementation commitments to this change

## 3. Hermes — docs-site UX, build, and deploy polish

- [x] 3.1 Update the docs-site landing flow (`index`, quick start, roadmap, and navigation) so status, install, release, coverage, and contribution paths are easy to find
- [x] 3.2 Adjust the docs workflow so pull requests validate the site build and `main` deploys the published site only after a successful build
- [x] 3.3 Verify the Astro/Starlight site still renders correctly on GitHub Pages with the repository-relative base path after the navigation/content updates

## 4. Zapp — public release surface and launch review

- [x] 4.1 Define the release-facing checklist covering asset names, checksum wording, install guidance, and deferred distribution channels
- [x] 4.2 Review README and docs-site copy for public launch clarity: what ships now, what is still planned, and where users should get binaries
- [x] 4.3 Confirm the GitHub Release notes/template language does not promise npm publishing or one-command installer UX in this slice

## 5. Cross-checks and reviewer gates

- [x] 5.1 Kif reviews the final coverage/release plan for free availability, artifact stability, and drift against the release workflow
- [x] 5.2 Scruffy verifies the coverage outputs and docs updates are inspectable from GitHub without relying on paid tooling
- [x] 5.3 Leela confirms all four spec files are reflected in the final implementation plan and that deferred benchmark/npm work is not silently pulled back into scope
