# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- The project wants a world-class open-source docs website, not just acceptable docs.
- Docs-site changes also follow the OpenSpec-first workflow when meaningful.
- Examples, IA, and search UX matter as much as prose quality.
- The `docs.yml` workflow previously only deployed on push to `main` — PRs had no build validation. Adding a `pull_request` trigger with a conditional `upload-pages-artifact` step (skipped on PRs) and a `deploy` job gated on `push`/`workflow_dispatch` is the correct GitHub Pages pattern.
- Starlight/Astro handles the repository-relative base path automatically when `GITHUB_ACTIONS=true` and `GITHUB_REPOSITORY` are set — all asset/link URLs pick up `/gigabrain/` without per-page changes.
- An "Install & Status" page is the clearest single anchor for surfacing supported-now vs planned-later distribution channels. It belongs first in the Getting Started nav group.
- Roadmap and README must agree on Phase status. When they diverge, README wins as the more actively-maintained source of truth.
- **Dual-release channel naming:** `airgapped` and `online` are the canonical channel names. "slim" may appear as an informal size descriptor but must not be used as a channel name/label anywhere in public docs.
- **Source-build default is `online`**: `Cargo.toml` sets `default = ["bundled", "online-model"]`. All docs showing `cargo build --release` should label it as the online (default) channel. Airgapped requires explicit `--no-default-features --features bundled,embedded-model`. Installer defaults (shell → airgapped, npm → online) are separate from the source-build default and are correct as documented.

## 2026-04-15 P3 Release — Docs-Site Polish & Completion

**Role:** Docs-site navigation, install pages, build/deploy workflow

**What happened:**
- Hermes created dedicated `guides/install.md` page as the primary navigation anchor for status/install matrix. Reordered homepage hero to "Install & Status" primary CTA (vs. old "Get Started").
- Added `pull_request` trigger to `docs.yml` so PRs validate the Astro build before merge (deploy still gated on `push`/`workflow_dispatch`).
- Corrected docs-site roadmap Phase 1 status from "In progress" to "Not started" to match README.
- Verified GitHub Pages base path is correct — all assets/links resolve under `/gigabrain/`.

**Outcome:** P3 Release docs-site component **COMPLETE**. Install page as primary CTA, PR validation on docs PRs, status aligned with README, all gates passed.

**Decision notes:** `.squad/decisions.md` (merged from inbox) — documents five docs-site polish decisions (install anchor, homepage CTA, PR validation, roadmap sync, base path verification).

## 2026-04-17 Phase 3 Final — v1.0.0 Docs, Archival, PR

**Role:** Docs-site Phase 3 completion, OpenSpec archival, PR creation

**What happened:**
- Updated homepage, install page, roadmap, getting-started, quick-start to reflect Phase 3 complete and v1.0.0 release-ready state.
- Created new guide `guides/phase3-capabilities.md` — skills, validate, call, pipe, benchmarks, Phase 3 MCP tools — added to Astro sidebar.
- Promoted Phase 3 MCP tools from stub "Other tools" note to full documented table + call examples in mcp-server guide.
- Removed stale "Planned API" callout from CLI reference; all commands now implemented.
- Updated README: Phase 3 complete, "Planned features" → "Features", install table shows v1.0.0 available, Contributing section updated.
- Archived both `p3-polish-benchmarks` and `p3-skills-benchmarks` to `openspec/changes/archive/2026-04-17-*`; marked both `status: complete`.
- Verified docs site build: 15 pages, zero errors.
- Pushed branch and created PR #31: "Phase 3: Skills, Benchmarks, CLI Polish, and v1.0.0 Docs".

**Outcome:** Phase 3 docs-site component **COMPLETE**. v1.0.0 status accurate across all public surfaces. All Phase 3 proposals archived. PR ready for Professor + Nibbler review.

**Decision notes:** `.squad/decisions/inbox/hermes-phase3-site.md` — five decisions: Phase 3 guide as standalone page, simultaneous archival, "Planned API" removal, Phase 3 MCP tool promotion, README section rename.

## 2026-04-18 v0.9.1 Dual-Release Lane — Docs-Site Consistency Pass

**Role:** Docs-site engineer, dual-release channel alignment

**What happened:**
- Identified that `Cargo.toml` default features are `["bundled", "online-model"]` — source-build default is the **online** channel, not airgapped. All docs were previously inverted on this point.
- Corrected `install.md`, `getting-started.md`, `contributing.md`: source-build default now correctly stated as **online**; airgapped now shows correct explicit feature flags (`--no-default-features --features bundled,embedded-model`).
- Fixed `spec.md` embedded Cargo.toml snippet (`default = ["bundled", "online-model"]`) and both build command blocks to match actual Cargo defaults.
- Normalized "slim online" compound label and "online slim" variants to clean approved channel names (`airgapped` / `online`). Retained "slimmer" as an acceptable informal size descriptor where it appeared as an adjective, not a channel name.
- Verified docs site build: 15 pages, zero errors.

**Outcome:** Docs site now truthful and internally consistent on the dual-release contract. Source-build default aligned with Cargo defaults (online). Installer defaults (shell → airgapped, npm → online) remain correct and unchanged.
