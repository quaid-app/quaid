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
- For spec-backed CLI work, verify the exact command shape end-to-end; matching help text and docs is insufficient if the advertised subcommand path does not parse.
- Re-reviewing a CLI wiring fix still requires both parser-path confirmation in source and live command execution; one without the other can miss dispatch regressions.

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

## 2026-04-16: Phase 3 Core Review — Rejection (tasks 8.1)

**Scope:** validate.rs, skills.rs, call.rs, pipe.rs, Phase 3 MCP handlers  
**Status:** Completed with REJECTION  

**Blocked artifacts:**
1. `src/commands/validate.rs` — missing stale-vector resolution check
2. `src/commands/skills.rs` — incorrect embedded-vs-local skill resolution

**Acceptable:** call.rs dispatch, pipe.rs continuation, Phase 3 MCP tools  

**Decision:** professor-phase3-core-review.md merged to decisions.md  
**Task 8.1:** Not marked complete; revision author must address blockers and resubmit.
