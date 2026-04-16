# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- The team wants high unit-test coverage, not token test presence.
- Proposal-first work helps define the invariants tests must guard.
- Coverage depth is a first-class role in this squad.
- Coverage review is not just artifact existence; README/docs must point to the same free GitHub coverage surface, and status wording must stay synchronized across repo docs.
- Benchmark reproducibility review needs two layers: rerun offline gates twice for stable pass/fail behavior, then verify the pinned-data story is real (lockfile consumed, hashes finalized, baselines established).
- For CI-gated benchmark reproducibility, compare pass/ignore counts across reruns and treat wall-clock durations as noise unless the harness explicitly gates latency.
- `scripts/install.sh` must stay LF-normalized; CRLF line endings break POSIX `sh` execution under WSL before any installer logic runs.
- Cross-platform installer validation from Windows works best with repo-scoped `TMPDIR` paths in WSL plus PATH-injected fake tools for shell error branches, and small Node harnesses with platform overrides for `postinstall.js` network branches.
- `packages/gbrain-npm/package.json` currently targets the unscoped public package name `gbrain`, but npm already serves `gbrain@1.3.1`, so `npm publish --dry-run` rejects `gbrain@0.9.0` before token handling.
- `packages/gbrain-npm/scripts/postinstall.js` currently hardcodes `v${package.version}` as its release tag target; with no `v0.9.0` GitHub Release present in `macro88/gigabrain`, supported-platform installs would currently 404.

## 2026-04-16 Phase 3 — Benchmark Reproducibility Review (Task 8.4)

- Re-ran the new offline Rust benchmark/test paths twice: `beir_eval` unit slice, `corpus_reality`, `concurrency_stress`, `embedding_migration`, plus `benchmarks/prep_datasets.sh --verify-only`.
- Result: the runnable Rust paths were behaviorally stable across both passes; only acceptable variance was wall-clock duration and interleaving of `Embedded ... chunks` log lines under test scheduling.
- Rejected task 8.4 anyway because the reproducibility contract is still incomplete: `benchmarks/datasets.lock` still carries placeholder hash/update notes, `prep_datasets.sh` advertises lockfile-driven pins but hardcodes values instead of reading the lock, and BEIR baselines remain `null`/`pending`, so identical benchmark scores cannot yet be verified end-to-end.

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

## 2026-04-15T03:48:02Z Phase 1 missing tests completed (T01, T22, T27, T28)

- Added `core::types` serde round-trip coverage for `Page`, asserting slug/title/version plus tags stored in `frontmatter` survive JSON serialization.
- Extended `core::migrate` coverage with the SHA-256 idempotency branch: after importing a copied fixture corpus, mutating one fixture causes exactly one re-import and the remaining files are skipped.
- Added `src/lib.rs` so integration tests can reach crate modules from `tests/`.
- Added `tests/roundtrip_raw.rs` with a constructed canonical markdown fixture for byte-exact export verification.
- Added `tests/roundtrip_semantic.rs` to verify import -> export -> re-import preserves page count and normalized exported markdown hashes across the canonicalized corpus.
- Added MCP server unit coverage for tools capability exposure, not-found mapping (`-32001`), and OCC conflict mapping (`-32009` with `current_version`).
- Pattern note: semantic round-trip needs normalized line endings because some fixtures carry CRLF timeline bytes through the first export before canonical re-import collapses them to LF.

## 2026-04-15 P3 Release — Inspectability Gate Review & Re-review & Approval

**Role:** P3 Release gate review (task 5.2 coverage inspectability)

**What happened:**
- Scruffy's initial review (task 5.2) verified coverage outputs are free and GitHub-visible (lcov.info artifact + job summary), but identified two blocking issues: coverage surface not documented in public docs, README/docs-site status messaging still drifts.
- Marked task 5.2 blocked with specific doc revision requirements. Amy added coverage guidance to README/docs pages pointing to GitHub Actions surface and stating coverage is informational. Hermes synced docs-site roadmap/status with README.
- Re-reviewed after fixes. Both doc accuracy issues resolved. Task 5.2 **APPROVED**.

**Outcome:** P3 Release gate 5.2 (inspectability) **COMPLETE & APPROVED**. Coverage surface documented and GitHub-visible, status messaging aligned across all surfaces, sign-off complete.

**Decision notes:** `.squad/decisions.md` (merged from inbox) — documents Scruffy's task 5.2 review, blocking issues, and re-review approval.

## 2026-04-15 Cross-team Update

- **Professor completed graph parent-aware tree rendering** (commit `44ad720`). Multi-hop depth-2 edges now render beneath actual parent instead of flattening under root. Depth-2 integration test strengthened with exact text shape assertions. All validation gates pass.
- **Fry advancing slices:** Progressive retrieval (tasks 5.1–5.6) and assertions/check (tasks 3.1–4.5) both implemented. All 193 tests pass (up from 185). Decisions merged into canonical ledger. Awaiting Nibbler's final graph re-review and completion.

## 2026-04-16T14:59:20Z Simplified-install v0.9.0 Release — Scruffy Completion

- **Task:** Validated installer and package paths, normalized line endings, updated task documentation, added validation skill
- **Changes:**
  1. Installer path validation — confirmed `simplified-install/` paths and script locations
  2. Package paths normalization — verified `scripts/install.sh` and Windows/Unix consistency
  3. Line endings normalization — updated `scripts/install.sh` to consistent CRLF/LF handling
  4. Task documentation — updated `simplified-install/tasks.md` with validation guidance
  5. Validation skill — created/appended skill documentation for install validation
- **Status:** ✅ COMPLETE. Installer paths validated and documented. scripts/install.sh ready for v0.9.0 release.
- **Orchestration log:** `.squad/orchestration-log/2026-04-16T14-59-20Z-scruffy.md`
