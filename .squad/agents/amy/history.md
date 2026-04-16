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
