# Hermes History Archive

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

## 2026-04-18 (revision) — Bender Rejection: Source-Build Default Correction

**Role:** Docs revision owner assigned by Bender rejection

**What happened:**
- Bender rejected the v0.9.1 dual-release branch: Cargo.toml `default = ["bundled", "embedded-model"]` means `cargo build --release` = airgapped. Docs (and a previous history entry) claimed the opposite.
- Hermes corrected all 9 affected doc surfaces: README, CLAUDE.md, docs/getting-started.md, docs/contributing.md, docs/spec.md, and all four matching website docs.
- Explicit online build command corrected to `--no-default-features --features bundled,online-model` throughout.
- Installer defaults untouched: shell installer → airgapped, npm → online.
- Committed as single reconciliation commit on `release/v0.9.1-dual-release`.

**Outcome:** All doc surfaces now match the approved contract from task A.4. Bender rejection addressed.

## 2026-04-19: Dual Release v0.9.1 — Session Completion

**Role:** Scribe session logger and decision merger

**Summary:** Completed dual-release v0.9.1 cycle:
- Leela: OpenSpec cleanup (removed duplicate, populated tasks.md, confirmed A.1–C.3 done)
- Fry: Full implementation (Cargo + npm + CI + installer, all phases A–C)
- Amy: Docs Phase C (flagged HIGH defect: Cargo default vs. docs mismatch)
- Hermes: Docs Phase 1 (corrected online → airgapped after Bender rejection)
- Bender: Two validation rounds (D.1 rejected for HIGH defect, D.1 rereview approved)
- Coordinator: Pushed branch, opened PR #33

**Outcome:** All tests pass, release contract coherent, PR #33 open and ready to merge.

## 2026-04-25: Public Docs Refresh — vault-sync-engine promotion-ready pass

**Role:** Docs-site engineer, promotion-readiness pass

**What happened:**
- Identified and fixed six stale/incorrect docs-site surfaces.
- `index.mdx` homepage: replaced fake `"Server active at http://localhost:8080"` terminal output with an accurate three-command snippet (`init` → `import` → `serve`). GigaBrain serve is stdio, not HTTP.
- `install.mdx`: updated version pins `v0.9.2` → `v0.9.4` in both the GitHub Releases download snippet and all `GBRAIN_VERSION` installer examples.
- `getting-started.mdx`: updated "v4 schema" → "v5 schema" in step 01; updated "sixteen production tools" → "seventeen production tools" in the action banner.
- `contributing.md`: updated repo layout `schema.sql` annotation "v4 DDL" → "v5 DDL".
- `phase3-capabilities.md`: updated "16 MCP tools" → "17 MCP tools" in the `call` command description and the Related link at the bottom.
- `mcp-server.md`: added `brain_collections` as a new vault-sync-engine tool table entry with a full 13-field response shape example. Added `## Phase 3 tool examples` header back after the vault-sync section.
- `roadmap.md`: added a full vault-sync-engine section listing landed capabilities and explicitly noting restore/IPC as deferred. Cleaned up v0.9.4 version targets row (removed internal #55 issue reference).
- Verified docs site build: 15 pages, zero errors.

**Outcome:** Docs site is now promotion-ready for what is actually landed. All tool counts accurate (17). Restore and IPC are explicitly deferred in the roadmap — not advertised. Decisions written to `.squad/decisions/inbox/hermes-public-docs-refresh.md`.

## 2026-04-25: PR #77 Docs Review — vault-sync-engine accuracy pass

**Role:** Docs-site engineer, PR review resolution + release ship

**What happened:**
- Addressed all 20+ review comments on PR #77 across 4 docs-site files.
- `mcp-server.md`: replaced fabricated `brain_collections` JSON example with the real `BrainCollectionView` struct fields; corrected `state` enum (`needs_sync` → `detached`); fixed table description (removed "watcher activity, blocker").
- `roadmap.md`: moved quarantine restore from "Explicitly deferred" to Landed with Unix-only caveat; updated `brain_collections` bullet to match real field list.
- `index.mdx`: bumped tool count 16→17; added Unix/macOS/Linux note to `gbrain serve` quickstart comment and Live Vault Sync card.
- `why-gigabrain.mdx`: fixed grammar nit ("so the brain an AI agent reads" → "so the brain that an AI agent reads"); added Unix-only note to vault-watcher paragraph.
- Verified docs site build: 15 pages, zero errors.
- Committed and pushed to `spec/vault-sync-engine`; marked PR ready; merged with `--admin`; tagged and released `v0.9.6`.

**Outcome:** All review comments addressed. PR #77 merged to main. v0.9.6 released at https://github.com/macro88/gigabrain/releases/tag/v0.9.6.
