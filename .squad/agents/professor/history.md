# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Review work should start from the proposal and the accepted project constraints.
- This team expects explicit reviewer gating, not silent approval.
- Maintainability and architectural coherence are key review criteria.

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
