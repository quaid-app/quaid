# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Review work should start from the proposal and the accepted project constraints.
- This team expects explicit reviewer gating, not silent approval.
- Maintainability and architectural coherence are key review criteria.
- For CLI review, validate behavior from more than one working directory; path-dependent “embedded” resources can look correct at repo root while failing the shipped-binary contract.
- A schema-foundation slice is not landable if it bumps required page fields or uniqueness rules without updating downstream insert/query callsites and quarantine filters; `cargo check` can stay green while runtime tests collapse.
- A repair pass is still rejectable when `tasks.md` is updated but proposal/design artifacts continue to describe a different contract; reviewer truthfulness is proposal-first, not task-note-first.
- Full green tests do not clear a schema slice if legacy-open paths still mutate an old database before refusing it; preflight safety must happen before any v5 DDL side effects.
- When a foundation slice keeps a temporary compatibility shim, proposal/design text must say so explicitly; a repair note in tasks.md is not enough to clear truthfulness review.
- A batch can be truthful about partial implementation and still be rejectable if a public scaffold on a safety-critical path returns benign success values instead of making deferral explicit.
- For vault-sync work, stubbed reconciler entry points should fail loudly or stay clearly unwired; returning empty stats or `false` for DB-only-state checks is too easy to mistake for real behavior.


## Core Context

**Historical Summary (2026-04-13 to 2026-04-21):**
- Phase 1 review leadership: Set review bar for truthfulness, maintainability, coherence
- T14–T19 rejection: Exposed inference shim semantic contract drift, embed CLI mixed-mode, build breakage
- Phase 2 graph re-review: Approved after directionality/output/coverage blockers cleared
- Vault-sync foundation third-pass: Identified legacy-open safety gap (v5 DDL before version check), schema compatibility shims, quarantine filtering consistency

**Review Standards Established:**
- Artifact truthfulness: Proposal/design must match implementation exactly
- Safety-first architecture: Preflight checks before mutations; explicit error semantics for safety-critical paths
- Coverage depth: 90%+ required; foundation slices need direct unit tests, not just e2e validation
- Multi-dimensional review: Truthfulness + performance + coverage; don't conflate test-pass with gate-pass
## 2026-04-14 Update

- Fry completed rust-best-practices skill adoption recommendation. Skill recommended for all Rust implementation and review work. Key alignment: error handling split matches our practice, CLI discipline aligns with CI gates, performance constraints match single-binary target.
- MCP evaluation still pending. Coordinator has flagged GitHub MCP as the only currently useful integration for this repo in interim.
- Team memory synchronized: decisions inbox merged into canonical ledger, orchestration logs written, team coordination complete.
- The Rust handbook at `.agents/skills/rust-best-practices/` is adoptable as standing guidance only if rules are classified into defaults vs optional techniques.
- For GigaBrain, strong Rust defaults are borrow-over-clone, `Result` over panic, measured performance work, and justified `#[expect(clippy::...)]` instead of blanket lint suppression.
- GigaBrain is currently a binary crate with internal modules (`src/main.rs`, `src/commands/`, `src/core/`), so library-only rules like blanket `#![deny(missing_docs)]` should stay conditional rather than repo-wide policy.

## 2026-04-14T03:59:44Z Scribe Merge (T03 completion)

- Reviewed T03 markdown slice completion and Scruffy's test strategy.
- T03 decisions merged into canonical `decisions.md`: frontmatter canonical order (alphabetical sort for deterministic render), timeline sep omit-when-empty, YAML parse graceful degradation, non-scalar YAML skip.
- Rust skill adoption finalized in team memory with caveats section (MSRV ≥1.81 for `#[expect]`, nightly-only for rustfmt import grouping, snapshot testing deferred to Phase 1).
- Cross-agent history updated. Orchestration and session logs written. Inbox cleared.

## 2026-04-14 Search/Embed Review

- Phase 1 search review outcome: reject for landing until semantic search stops pretending the SHA-based embedding shim is the specified Candle/BGE implementation.
- Embed CLI review rule clarified: `gbrain embed [SLUG | --all | --stale]` must behave as an explicit mode surface, not a permissive mixture that silently ignores flags.
- Review bar reaffirmed: a slice under active signature churn is not review-complete if targeted `cargo test` compilation no longer passes.

## 2026-04-14T04:56:03Z Phase 1 T14–T19 Code Review (REJECTION)

- Reviewed Fry's T14–T19 submission (commit 2d5f710) for Phase 1 search/embed/query surface.
- **Verdict: REJECTION FOR LANDING** on three blocking grounds:
  1. **Inference shim semantic contract drift:** `inference.rs` public API promises BGE-small-en-v1.5 embeddings but delivers SHA-256 hash shim. No candle-* wiring, no embedded weights, no feature gates. Vector similarity scores are hash-proximity. Semantic guarantees misleading.
  2. **Embed CLI mixed-mode allowed:** Contract says `gbrain embed [SLUG | --all | --stale]` (mutually exclusive). Implementation allows mixed modes (`SLUG + --all`), silently privileges slug path instead of rejecting. Also, `--all` re-embeds instead of skipping unchanged content per spec.
  3. **Tests fail compilation:** `embed::run` signature changed to 4 args. Old test callsites still call 3-arg form. `cargo test` fails before review can proceed.
- **Non-blocking note:** `--depth` flag exposed but ignored; help text should clarify deferral or remove from Phase 1 surface.
- FTS implementation (T13) itself is acceptable. Rejection is on semantic-search truthfulness, embed CLI contract integrity, and build breakage.
- Recommended: Fry address blockers and resubmit, or defer semantic search blocker to Phase 2 if time-bound.
- Orchestration log written: `2026-04-14T04-56-03Z-professor-rejection-findings.md`.
- Leela revision cycle assigned (independent of Fry). Outcome: APPROVED with explicit placeholder caveats + stderr warnings + honest status notes (5 decisions). All 115 tests pass unchanged. Approved for Phase 1 ship gate.

## 2026-04-15 SG-3 / SG-4 / SG-5 Sign-off

- Reviewed the Phase 1 proposal/design before execution, then re-verified gates directly against the tree and binaries.
- SG-3 APPROVED: import/export/re-import roundtrip over `tests/fixtures/` preserved page count and slug set exactly (5 pages, zero semantic diff by gate definition).
- SG-4 APPROVED: `src/mcp/server.rs` exposes exactly the 5 Phase 1 tools; `cargo test mcp` passed; live stdio session completed `initialize`, `tools/list`, and `tools/call` for all 5 tools.
- SG-5 APPROVED: `target/x86_64-unknown-linux-musl/release/gbrain` exists and is genuinely static (`file`: `static-pie linked`; `ldd`: `statically linked`).

## 2026-04-15 Phase 2 Graph Slice Re-review (Final)

- **Status:** RE-REVIEW COMPLETE — APPROVED FOR LANDING
- **Scope:** OpenSpec `p2-intelligence-layer` graph slice only (tasks 1.1–2.5; `src/core/graph.rs`, `src/commands/graph.rs`, `tests/graph.rs`)
- **Timeline:** Initial rejection (prior to phase2 kickoff); Leela revision completed; re-review 2026-04-15
- **Verdict:** All three blockers from the prior rejection are now resolved:
  1. **Directionality contract:** `neighborhood_graph` confirmed outbound-only per spec. `gbrain backlinks` remains inbound surface. Command/API surfaces are now orthogonal.
  2. **Human-readable output:** CLI prints `→ <edge.to> (<relationship>)` over outbound-only result set only. Root no longer appears as self-neighbour.
  3. **CLI test coverage:** `run_to<W: Write>` refactor makes output injectable. Integration tests now capture actual text and `--json` output shape.
- **Temporal gate update:** D2 from Leela revision confirmed in contract — active filter now gates both `valid_from <= date('now')` and `valid_until >= date('now')`. Mom's future-dated link edge case is now covered.
- **Scope caveat:** This approval is for graph slice tasks 1.1–2.5 only. Issue #28 progressive-retrieval budget/OCC review lane remains separate and not re-opened.
- **Validation:** `cargo test graph --quiet` ✅, `cargo test --quiet` ✅, `cargo clippy --quiet -- -D warnings` ✅, `cargo fmt --check` ✅
- **Decision:** APPROVE FOR LANDING to `phase2/p2-intelligence-layer`.

## 2026-04-15 Cross-team Update

- **Scruffy completed graph cycle/self-loop render suppression** (commit `acd03ac`). Self-edges and cycles no longer print root back into human output. Traversal termination unchanged; human-facing contract now matches spec. All validation gates pass.
- **Fry advancing slices:** Progressive retrieval (tasks 5.1–5.6) and assertions/check (tasks 3.1–4.5) both implemented. All 193 tests pass (up from 185). Token-budget logic and contradiction dedup verified. Awaiting Nibbler's final graph re-review before Phase 2 sign-off.

---

## 2026-04-22 Vault-Sync Foundation Third-Pass Review

**Status:** REPAIR DECISION ISSUED

**Scope:** Vault-sync-engine foundation slice (tasks 1.1–2.6) repaired by Leela for schema coherence, legacy compatibility, and test failures.

**Work Performed:**

1. **Artifact Cross-Validation:** Re-read proposal/design against implementation to detect overstated removals
   - Proposal still describes `gbrain import` and `ingest_log` as removed
   - Implementation retains both as temporary compatibility shims
   - This is a valid technical choice, but artifacts must be explicit

2. **Legacy-Open Safety Audit:** Traced schema version checking in `db.rs`
   - `open_with_model()` calls `open_connection()` which executes v5 DDL BEFORE version check
   - Pre-v5 databases can be partially mutated before re-init refusal
   - Preflight safety must happen before ANY v5 execution

3. **Coverage Depth Assessment:** Identified three new branchy seams without direct tests
   - Collection routing matrix (`parse_slug()` with 6 operation types)
   - Quarantine filtering (quarantined pages excluded from vector search)
   - Schema refusal branch (pre-v5 brains rejected before mutations)

**Findings:**

- ✅ Repairs resolve 181 prior test failures (cargo test now passes)
- ✅ Legacy write paths work with new schema
- ✅ Quarantine filtering wired through vector search
- ⚠️ Proposal/design truthfulness — GATE 1
- ⚠️ Legacy-open safety reordering — GATE 2
- ⚠️ Coverage depth for new seams — GATE 3

**Required Before Landing:**

1. **Gate 1:** Align proposal/design with actual transitional contract (keep shims OR remove now)
2. **Gate 2:** Reorder schema gating: version check before ANY v5 DDL
3. **Gate 3:** Add three focused unit-test groups for new seams

**Key Learning:** Schema foundation slices cannot land with green tests alone. Truthfulness (proposal vs implementation), safety (preflight gating), and coverage depth (new seams directly tested) are co-equal gates.

**Verdict:** REPAIR DECISION ISSUED. Three gates remain before landing.

**Decision Artifacts Merged:**
1. Vault-Sync Foundation Review Gating — three-gate policy for future review passes
2. Coverage Depth Review — Scruffy assessment of new branchy seams
3. Professor re-review — artifact truthfulness + safety + coverage findings

**Next Steps:** Leela addresses gates 1–3; resubmits; Professor conducts final review.



## 2026-04-16: Phase 3 Core Review — Rejection (tasks 8.1)

**Scope:** validate.rs, skills.rs, call.rs, pipe.rs, Phase 3 MCP handlers  
**Status:** Completed with REJECTION  

**Blocked artifacts:**
1. `src/commands/validate.rs` — missing stale-vector resolution check
2. `src/commands/skills.rs` — incorrect embedded-vs-local skill resolution

**Acceptable:** call.rs dispatch, pipe.rs continuation, Phase 3 MCP tools  

**Decision:** professor-phase3-core-review.md merged to decisions.md  
**Task 8.1:** Not marked complete; revision author must address blockers and resubmit.

---

## 2026-04-22: Vault-Sync Foundation Rejection → Repair Cycle

**Session:** Professor review → Leela repair of vault-sync-engine foundation slice.

**What happened:**
- Reviewed Leela's vault-sync foundation slice for schema v5 + collections module coherence.
- **Verdict: REJECTION FOR NEXT BATCH** on four blocking grounds:
  1. Task completion overstated in tasks.md vs actual schema
  2. Legacy schema version handling unsafe (executes v5 DDL before legacy check)
  3. Schema changes not integrated with existing write paths (181 test failures in full `cargo test`)
  4. Foundations not yet maintainable (missing quarantine filtering in search_vec, incomplete validator coverage)
- Recommended Leela take ownership of integration-focused repair pass rather than Fry rewriting.

**Repair outcome (Leela completed):**
- All four blockers resolved in coordinated repair:
  1. tasks.md updated to reflect actual schema state (1.1, 1.6, 2.6 marked pending)
  2. Schema version gating fixed: legacy check now runs BEFORE v5 DDL execution
  3. All 20+ legacy write sites now work with NEW unique constraint via `DEFAULT 1` + `ensure_default_collection()`
  4. Quarantine filtering wired through `search_vec` (FTS5 already had it)
- Result: 181 test failures → **0 failures**, foundation ready for follow-on implementation.

**Review lesson:** Rejection + repair cycle is faster than rewrite when the core issue is integration (wiring paths, not design). Gave Leela clear blocker list → she fixed atomically → no rework needed. Schema v5 foundation now coherent and test-clean.

## 2026-04-22 Batch B Narrow Repair Gate Clear

**Session:** Scribe decision merge + Leela narrow repair completion logging

**Review outcome:**
- Professor's gating feedback on Batch B (safety-critical reconciler semantics + documentation accuracy) was resolved via focused repair pass by Leela.
- Repair scope: strict reconciler scaffold surface (reconciler.rs, tasks.md). No Batch C logic, no expand of approved groups.
- Safety semantics fix: has_db_only_state() now returns explicit Err instead of Ok(false), forcing caller error handling.
- Documentation fix: module header now accurately describes "will replace" (future) vs. "replaces" (completed).

**Decision ledger:**
- Leela's three repair decisions merged to canonical decisions.md (gate decision, repair decision, original review decision now in record)
- Decisions inbox cleared; Scribe orchestration/session logs written

**Batch B status:** ✅ Gate clean, ready for Batch C implementation planning. Professor can now sign off on Group 3 (ignore_patterns), Group 4 (file_state), and Group 5.1 scaffold landing.

