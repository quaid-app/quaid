# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- The team wants high unit-test coverage, not token test presence.
- Proposal-first work helps define the invariants tests must guard.
- Coverage depth is a first-class role in this squad.

## 2026-04-14T03:59:44Z Scribe Merge (T03 completion)

- Scruffy's T03 markdown test strategy merged into canonical `decisions.md`.
- 20+ must-cover test cases locked before Fry writes parsing logic (prevent re-litigation per-function).
- Test expectations organized by function (parse_frontmatter, split_content, extract_summary, render_page) with 4-5 must-cover cases each.
- Fixture guidance provided: canonical, boundary-trap, no-frontmatter.
- Critical implementation traps documented: HashMap order nondeterminism, trim() fidelity loss, type coercion underspecification, dual `---` roles.
- Orchestration log written. Inbox cleared. Cross-agent histories updated.

## 2026-04-14T04:07:24Z Phase 1 T06 put Command Coverage Spec

- Locked comprehensive unit test specification for T06 put command before code lands.
- Three core test cases frozen: create (version 1), update (OCC success), conflict (OCC failure).
- Implementation seam specified: pure helper function + thin CLI wrapper (enables deterministic unit coverage).
- Four assertion guards documented: frontmatter comparison, markdown split fidelity, OCC semantics, Phase 1 room behavior.
- Test naming convention provided (4 test names in kebab-case).
- Status: BLOCKED — implementation not ready; coverage plan locked.

## 2026-04-14T04:07:24Z Scribe Merge (T05, T07, T03 approval, T06 spec)

- Scribe wrote 3 orchestration logs (Fry T05+T07, Bender T03 approval, Scruffy T06 spec).
- Scribe wrote session log for Phase 1 command slice window (3h execution).
- Four inbox decisions merged into canonical decisions.md (no duplicates found).
- Inbox files deleted after merge.
- Cross-agent history updates applied.
- Ready for git commit.

## 2026-04-14T05:24:00Z T20 novelty coverage locked

- Reworked `src/core/novelty.rs` around deterministic branch contracts: lexical duplicate threshold first, embedding near-duplicate fallback second.
- Added three focused unit tests for the requested invariants: identical content rejected, clearly different content accepted without embeddings, clearly different content accepted even with stored placeholder embeddings.
- Switched novelty tests to `db::open(":memory:")` so the coverage stays deterministic and avoids filesystem scratch state.
