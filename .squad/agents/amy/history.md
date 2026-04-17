# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Docs must make a sophisticated local-first system feel approachable.
- OpenSpec proposals are part of the writing input, not just implementation input.
- The docs goal is excellent onboarding and reference quality.
- Always define and apply a single status/install matrix across README and all docs pages at once — drift between surfaces confuses users and is hard to catch later.
- "Deferred follow-on" language for npm/installer must be explicit in both README and docs, not buried in a single footnote. A table showing supported-now vs. deferred reads faster than prose.
- When README and docs roadmap disagree on phase status (e.g., "not started" vs. "in progress"), the roadmap docs are usually more current — resolve by reading both sources before writing.
- Phase 3 gate in roadmap.md said `v0.1.0` when it should have been `v1.0.0` — easy to miss without cross-checking version targets table.
- When a phase ships in two OpenSpec slices, name both explicitly in the roadmap — a single "Phase 3" label masks which proposal delivered what, and contributors need to know which to read.
- Benchmark README can say "runs in CI" before the CI job is wired — always cross-check tasks.md for the CI wiring tasks before asserting that a gate is live.
- MCP tool count must be audited in docs whenever a phase adds tools; it drifts silently when the count is buried in prose rather than derived from a single authoritative table.

## 2026-04-15 P2 Docs Audit

**Role:** Pre-ship docs audit for Phase 2 (intelligence layer)

**What happened:**
- Audited all repo docs impacted by Phase 2 against the proposal, design, tasks, and five feature specs.
- Made safe implementation-independent updates: README.md (roadmap row, usage note, MCP tools split, contributing section), docs/roadmap.md (status → In progress, branch/PR reference), docs/getting-started.md (Phase 2 command callouts, MCP note), docs/contributing.md (Phase 2 reviewers added to gates table).
- Produced a full post-ship update map in `.squad/decisions/inbox/amy-phase2-docs.md` — exact per-file, per-location change map for README, roadmap, getting-started, contributing, spec.md, and proposal frontmatter.
- Left a comment on issue #26 summarizing the work.
- Did NOT update docs/spec.md (already accurate for Phase 2 design; needs post-ship verification pass only).

**Outcome:** Pre-ship pass COMPLETE. Post-ship pass gated on PR #22 merge + v0.2.0 tag push. Update map ready.

**Learnings:**
- Split "available MCP tools" into Phase 1 (shipped) and Phase 2 (in progress) when tools are phased — a flat list implies all tools are live.
- Getting-started tutorials that show Phase 2 commands need explicit phase callouts; tutorials read as "this works now" by default.
- The post-ship update map should be written at audit time (while the specs are fresh) — not after code lands.



**Role:** Public documentation refresh, install/status matrix owner

**What happened:**
- Amy refreshed `README.md` and created three new docs: `docs/getting-started.md`, `docs/roadmap.md`, `docs/contributing.md` as part of P3 release scope.
- Scruffy's review (task 5.2) rejected because coverage guidance was missing from README/docs pages — no pointer to GitHub Actions coverage artifact or job summary.
- Amy added coverage guidance to all public docs pages stating coverage is informational, not gating, and pointing readers to the GitHub Actions surface.
- After fixes, task 5.2 passed Scruffy's re-review.

**Outcome:** P3 Release docs component **COMPLETE**. README/docs aligned on status, install, coverage, and phase/version messaging. All gates passed.

**Decision notes:** `.squad/decisions.md` (merged from inbox) — Amy's three-file decision (getting-started, roadmap, contributing split) + final doc fix decisions.


## 2026-04-17 Phase 3 Docs Audit (final)

**Role:** Phase 3 documentation owner — README, docs/, benchmarks/README.md

**What happened:**
- Updated README.md: status badge (Phase 3 in progress), roadmap table (🔄 In progress with actual scope), install table (v1.0.0 target, Phases 1–3 complete), usage section (added validate/call/pipe/skills commands), MCP tools (12 → 16, added Phase 3 tools), skills section (production-ready note, skills list + doctor commands), contributing section (Phase 3 is the active phase).
- Updated docs/roadmap.md: Phase 3 block rewritten to name both OpenSpec slices, list completed items, and list pending ship-gate items.
- Updated docs/getting-started.md: status section (Phase 3 in progress), install table (v1.0.0), first-brain note (all phases complete), MCP tools (16), skills (production-ready), added full Phase 3 commands section (validate, call, pipe, skills).
- Updated docs/contributing.md: repository layout (added validate.rs, call.rs, pipe.rs, skills.rs), reviewer gates table (added Leela Phase 3 skills, Scruffy benchmark, Kif benchmark lanes), CI gate note (Phase 3 benchmark tests).
- Updated benchmarks/README.md: added CI wiring caveat to both offline gates section and BEIR regression gate section (tasks 7.1–7.2 still pending).
- Wrote 6 decisions to `.squad/decisions/inbox/amy-phase3-docs.md`.

**Outcome:** Phase 3 docs pass COMPLETE. All prose docs outside website/ are now consistent with the current branch state.

**Learnings (added above).**

**Role:** Skills author for Phase 3 — tasks 1.1 through 1.5

**What happened:**
- Rewrote all five stub SKILL.md files into production-ready documents: briefing, alerts, research, upgrade, enrich.
- Each skill follows the "thin harness, fat skills" principle — full workflows, command sequences, failure modes, and configurable parameters; no Rust code changes.
- Documented `brain_gap_approve` as an approval workflow dependency only (not a binary command), per the design doc's explicit deferral decision.
- Added `min_binary_version: "0.3.0"` to all five skill frontmatters so the upgrade skill can enforce compatibility.
- Stale alert threshold: used 30-day delta (timeline vs. truth) from the spec scenario rather than the 90-day raw age in the task description — logged this as a team decision for Fry to confirm.
- Marked tasks 1.1–1.5 as `[x]` in `openspec/changes/p3-skills-benchmarks/tasks.md`.
- Wrote decisions to `.squad/decisions/inbox/amy-phase3-skills.md`.

**Outcome:** Tasks 1.1–1.5 COMPLETE. Five skill files are production-ready with no stub markers.

**Learnings:**
- Production SKILL.md files need four things that stubs always omit: (1) configurable parameters table, (2) failure modes table, (3) exact command sequences an agent can follow without ambiguity, and (4) explicit statements about what the skill does NOT do automatically (e.g., no auto-accept of external data into compiled_truth).
- When spec and task description give different numbers for the same threshold (30 days vs 90 days), pick the one from the spec scenario and flag the discrepancy for the implementer — don't silently pick one.
- The two-phase store-then-extract pattern (raw_data first, compiled_truth second) should be consistent across all enrichment sources. Establish this as a doc convention early so it doesn't drift per-source.
- `brain_gap_approve` as a workflow dependency (vs. a real tool) is a subtle but important distinction that must be stated explicitly in the skill — agents will try to call it otherwise.

## 2026-04-19: v0.9.4 OpenSpec alignment — FTS robustness and assertion tightening

**Role:** OpenSpec artifact owner for two v0.9.4 lanes

**What happened:**
- Completed `fts5-search-robustness/tasks.md`: added Phase E.3 (Kif's benchmark validation commands with `benchmark_issue_check.db`) and Phase E.4 (DAB rerun checkpoint requiring zero crash/parse-error failures before the lane closes). Phase D already covered all four character classes (`?`, `'`, `%`, dotted-version).
- Aligned `assertion-extraction-tightening` to Kif's v0.9.4 triage: removed `#55` from `closes:` in proposal.md (now `related_issues: ["#55"]`); added Non-Goal bullet deferring the semantic-similarity gate pending rerun; replaced Design Decision 6 with a "deferred pending rerun" decision; rewrote tasks.md Phase C/E/F — removed the full Phase E semantic gate implementation (E.1–E.5), added C.5 (corpus-reality regression for the #38 case), and replaced Phase F with a lean Phase E verification that includes a #55 rerun gate (E.4) as a decision checkpoint, not an implementation task.
- Noted `fts-search-hardening/` stub as redundant in `.squad/decisions/inbox/amy-fts-search-hardening-duplicate.md` with an archival action item. Did not delete it.

**Outcome:** Both lanes are now apply-ready and aligned to Kif's benchmark triage. The assertion lane lands extraction tightening for #38; #55 is a rerun/decision hook, not a code task.

**Learnings:**
- When a triage decision says "rerun first, then decide," the right task form is a decision checkpoint (a checklist item that gates a future lane), not an implementation task. Tasks that implement something that might not be needed add cost and drift.
- A "deferred decision" in design.md is more useful than a deleted decision — it records the reasoning and the trigger condition for the future lane, so whoever picks up #55 next has context.
- Redundant OpenSpec stubs (empty .openspec.yaml only) should be noted in a decision inbox item rather than deleted, so archive cleanup can happen atomically when the superseding lane ships.


The default channel can differ from what earlier proposals assumed. Docs claiming "airgapped default" when Cargo actually defaults to `online-model` are a silent correctness bug that real users hit on first build.
- **"slim" is a former channel-name synonym, not just a size adjective.** Earlier proposals (dual-release-distribution) used "slim" as a channel name; the accepted contract (bge-small-dual-release-channels) uses "online". Any word that doubles as both an adjective and a historical channel name needs explicit audit whenever a rename lands — grep for the exact old term, not just the new one.
- **Embedded code snippets in spec.md drift independently from source.** spec.md contains a Cargo.toml `[features]` block and a Build section that can diverge silently from the live Cargo.toml. Include these in the audit checklist for any release that changes Cargo features or default channels.
- **The website and docs/ may drift in different directions after a crash.** In this pass the website was already correct while docs/ was stale. Future passes should always diff both surfaces against the approved design rather than assuming one is authoritative.
- **List the default channel first in two-bullet channel lists.** Readers internalize the first item as the default. Ordering airgapped first when online is the default would be misleading even if both are clearly labelled.

## 2026-04-17 Phase 3 / v0.9.1 Dual Release Docs Pass

**Role:** Phase 3 / v0.9.1 dual-release documentation correctness

**What happened:**
- Audited all input artifacts (README.md, CLAUDE.md, docs/getting-started.md, docs/contributing.md, docs/spec.md, packages/gbrain-npm/README.md) against the approved bge-small-dual-release-channels design.
- Found two classes of drift: (1) "slim" appearing as a channel-name synonym; (2) source-build docs claiming `cargo build --release` produces airgapped when Cargo.toml `default = ["bundled", "online-model"]` makes online the actual default.
- Fixed all six source-build instances: README.md (Quick start + Build sections), CLAUDE.md (Build section + Embedding model section), docs/getting-started.md (lede, install table, build commands, requirements prose), docs/contributing.md (build commands), docs/spec.md (Solution prose, embedded CLAUDE.md block, Cargo features snippet, Build commands, Embedding model section).
- Removed all channel-name uses of "slim" (leaving "slimmer" as a size adjective where natural).
- packages/gbrain-npm/README.md: already correct, no changes.
- website/src/content/docs/guides/getting-started.md: already correct, flagged for Hermes as an observation.
- Shell installer "airgapped by default" retained as correct per design spec Decision 3.
- Wrote 6 decisions to `.squad/decisions/inbox/amy-dual-release-docs.md`.

**Outcome:** v0.9.1 dual-release docs pass COMPLETE. All input artifacts now match the approved channel contract: online is the Cargo default; airgapped requires explicit feature flag; no "slim" channel names remain in user-facing docs.

## 2026-04-17: Dual Release v0.9.1 Documentation Phase 1

**Role:** Phase C documentation normalization for dual-release v0.9.1

**What happened:**
- Audited all repository prose documentation against the approved bge-small-dual-release-channels contract
- Aligned channel nomenclature: removed "slim" terminology; standardized to `airgapped`/`online` exclusively
- Preserved "airgapped by default" for shell installer (intentional per design spec Decision 3)
- Flagged embedded Cargo.toml snippet in docs/spec.md as potentially stale (needs verification after A.4 implementation task)
- Identified HIGH-severity defect: source-build documentation claims `cargo build --release` produces online channel, but Fry's A.4 implementation sets default to `embedded-model` (airgapped) — docs will need reconciliation after implementation is merged

**Outcome:** Phase C docs pass COMPLETE. Ready for implementation merge + post-merge reconciliation if A.4 changes the default.

**Learnings:**
- When implementation tasks change fundamental defaults (like A.4 flipping Cargo defaults), documentation changes that completed before that implementation task must be re-validated. There is no automatic triggering mechanism; this must be manually surfaced at review time.
- Descriptive English ("slimmer binary") is acceptable; contract terms ("slim channel") are not. The distinction needs explicit enforcement at review time.
- Source-build docs and shell installer docs describe different defaults (online vs. airgapped) which is correct per the design. Users must be aware both paths are legitimate but different.
