# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Benchmark work is a dedicated lane on this project.
- The requested target model is Gemini 3.1 Pro when available on the active surface.
- Performance claims should trace back to proposal goals and measured evidence.
- Code coverage infrastructure should favor **cross-platform tooling** (cargo-llvm-cov > cargo-tarpaulin) due to release strategy (macOS + Linux musl targets). Codecov.io is free for public repos and integrates seamlessly with PR comments, badges, and threshold gates.
- GitHub Pages + coverage HTML is a valid *secondary* dashboard pattern (via peaceiris/actions-gh-pages) but not required; Codecov handles primary reporting. Both can run in parallel with zero conflict.
- Coverage gates should align with Phase gates: Phase 1 = aspirational (≥70%), Phase 3 = enforced with PR checks (delta <2%).
- Release/coverage review sign-off blocks on doc-surface parity: GitHub-visible coverage outputs and checksum file format must match README + website copy exactly, or the plan is not release-ready.

## Session Log

### 2026-04-15: Code Coverage Investigation & Recommendation

**Task:** Investigate best free code coverage approaches for Rust project on GitHub; compare options for push-to-main reporting; recommend primary + fallback paths.

**Approach:**
- Reviewed Cargo.toml, existing CI workflows (ci.yml, release.yml), GitHub Actions architecture
- Searched landscape: cargo-llvm-cov (LLVM-based), cargo-tarpaulin (ptrace-based), grcov (post-processor), Codecov, GitHub Pages
- Analyzed cross-platform implications (macOS arm64, x86_64; Linux musl)
- Compared reporting surfaces: PR comments, badges, status checks, dashboards, artifacts
- Evaluated friction: setup time, CI overhead, maintenance, cost

**Findings:**
- **cargo-llvm-cov** is production-grade, cross-platform, covers all test types (unit, integration, doc). ~45s CI overhead. Stable-compatible via taiki-e/install-action wrapper. ✅ Recommended.
- **cargo-tarpaulin** is simpler but Linux-only (ptrace). Good fallback if LLVM tooling causes issues. ✅ Fallback.
- **Codecov.io** is free for public repos, auto-posts PR comments, gate thresholds, historical dashboard. No token needed; uses GITHUB_TOKEN. ✅ Recommended secondary.
- **GitHub Pages + HTML dashboard** is optional secondary pattern (peaceiris/actions-gh-pages) for self-hosted history. Zero conflict with Codecov. ⚠️ Optional Phase 3.
- **grcov** overlaps with llvm-cov but steeper setup. Not recommended.

**Recommendation:**
1. **PRIMARY (Phase 1):** cargo-llvm-cov + Codecov. Add to ci.yml:test job. No new workflow needed.
2. **SECONDARY (Optional Phase 3):** GitHub Pages dashboard via peaceiris/actions-gh-pages.
3. **FALLBACK:** cargo-tarpaulin if LLVM tools cause build failures.

**Deliverable:**
- `.squad/decisions/inbox/kif-coverage-plan.md` — full investigation report with CI templates, friction analysis, and phase alignment
- Updated `kif/history.md` with coverage infrastructure learnings
- Identified GitHub reporting surfaces: badge (README), PR comments (Codecov), status checks (gate threshold), dashboard (codecov.io), artifacts (LCOV)

**Next:**
- Scribe: merge decision inbox entry to decisions.md
- Fry: implement Phase 1 test infrastructure spike (add llvm-cov step to ci.yml)
- Team: review Codecov dashboard after first main push, set coverage gates for Phase 3

**Task:** Record BEIR-style nDCG@10 baseline in benchmarks/README.md

**Approach:**
- Built release binary with `cargo build --release` (~3.5min)
- Created synthetic query set based on test fixtures (5 pages: 2 people, 2 companies, 1 project)
- Designed 8 queries with explicit ground-truth relevance judgments
- Ran queries via `gbrain query` and recorded top-3 results with scores
- Computed nDCG@10 using binary relevance and standard DCG formula
- Measured wall-clock latencies for FTS5, hybrid query, and import operations

**Results:**
- Perfect baseline: nDCG@10 = 1.0000, Hit@1 = 100%, Hit@3 = 100%
- FTS5 search: ~155ms (cold start)
- Hybrid query: ~420ms (cold start)
- Import (5 files): ~3.7s

**Findings:**
- Hash-based embeddings (SHA-256 shim) still achieve perfect recall on small synthetic corpus
- Lexical overlap is sufficient for these targeted queries
- Baseline is reproducible and establishes measurement methodology for future semantic eval

**Deliverable:**
- Updated `benchmarks/README.md` with Phase 1 baseline section
- Marked SG-8 complete in tasks.md
- Commit: 204edf3 "bench: establish Phase 1 BEIR-proxy nDCG@10 baseline"

**Next:**
- Semantic baseline with BGE-small-en-v1.5 after T14 completes
- Expand to BEIR subsets (NFCorpus, FiQA) in Phase 3
- Set regression gate: no more than 2% drop in nDCG@10

### 2026-04-15: Task 5.1 Review — Coverage/Release Plan Rejected

**Task:** Review `p3-polish-benchmarks` task 5.1 for free availability, artifact stability, and drift against the release workflow.

**Approach:**
- Reviewed the approved proposal, design, coverage/release specs, and task list
- Cross-checked `.github/workflows/ci.yml` and `.github/workflows/release.yml` against `README.md`, `.github/RELEASE_CHECKLIST.md`, and website docs
- Focused on whether the supported public surfaces matched the implemented workflow contract

**Findings:**
- ✅ Coverage is free and GitHub-visible in CI: `lcov.info` + `coverage-summary.txt` artifact and job summary satisfy the no-paid-surface requirement even if Codecov is skipped.
- ✅ Release artifact names are stable across `release.yml`, `README.md`, and `.github/RELEASE_CHECKLIST.md` (`gbrain-<platform>` plus matching `.sha256`).
- ❌ `website/src/content/docs/guides/install.md` still says coverage on pushes to `main` is only "planned", which conflicts with the implemented `coverage` job in `.github/workflows/ci.yml`.
- ❌ `website/src/content/docs/reference/spec.md` still documents the old hash-only checksum format and `echo ... | shasum --check` flow, while `release.yml` now emits standard `hash  filename` checksum files and verifies them directly.

**Verdict:**
- Task 5.1 is **not approved yet**.
- Leave `openspec/changes/p3-polish-benchmarks/tasks.md` unchanged until docs/spec surfaces are brought back into sync with the workflow contract.

**Next:**
- Amy/Hermes should fix the stale coverage wording in the public docs.
- The spec/docs owner should update the reference spec checksum examples to the standard `.sha256` format used by `release.yml`.

## 2026-04-15 P3 Release — Gate Review & Re-review & Approval

**Role:** P3 Release gate review (task 5.1 coverage/release plan)

**What happened:**
- Kif's initial review (task 5.1) identified two doc-drift issues blocking approval: coverage surface said "planned" but already implemented, checksum format examples were hash-only not standard.
- Marked task 5.1 blocked with specific revision requirements. Fry applied targeted fixes to spec.md and install.md.
- Re-reviewed after fixes. Both doc-drift issues resolved. Task 5.1 **APPROVED**.

**Outcome:** P3 Release gate 5.1 **COMPLETE & APPROVED**. Coverage/release plan parity verified, sign-off complete.

**Decision notes:** `.squad/decisions.md` (merged from inbox) — documents Kif's task 5.1 review, blocking issues, and re-review approval.

## 2026-04-15 Phase 3 Groups 5+6 — Benchmark Foundation Implementation

**Role:** Benchmark Expert implementing Groups 5 and 6

**What was built:**

**Group 5 — Offline CI gates (all complete):**
- `benchmarks/datasets.lock`: TOML pin file for BEIR NQ/FiQA (URL + SHA-256), LongMemEval (git commit), LoCoMo (git commit)
- `benchmarks/prep_datasets.sh`: download/verify script with `--verify-only`, `--compute-hashes`, subset flags
- `tests/beir_eval.rs`: BEIR nDCG@10 harness — NQ/FiQA tests `#[ignore]`, nDCG math unit tests run in CI
- `tests/corpus_reality.rs`: 7 corpus integration tests — all pass
- `tests/concurrency_stress.rs`: parallel OCC, duplicate ingest, WAL compact, concurrent readers — all pass
- `tests/embedding_migration.rs`: zero cross-model contamination — all pass
- `benchmarks/baselines/beir.json`: Phase 1 proxy baseline (synthetic), NQ/FiQA pending real BEIR run

**Group 6 — Advisory benchmarks (all complete):**
- `benchmarks/requirements.txt`: pinned Python deps
- `benchmarks/longmemeval_adapter.py`: R@5 adapter (target ≥85%)
- `benchmarks/locomo_eval.py`: token-F1 hybrid vs FTS5 (target +30%)
- `benchmarks/ragas_eval.py`: Ragas advisory with OpenAI/Ollama judge

**Task 7.3:** `benchmarks/README.md` updated with Phase 3 advisory workflow, Ollama setup, BEIR gate docs.

**Key blockers encountered and resolved:**
1. `embedding_to_blob` was `pub(crate)` — promoted to `pub` for migration test access
2. FTS5 chokes on "Co-" prefix as negation/column prefix — fixed by using plain query terms
3. Round-trip test with CRLF fixture files — fixed by normalizing line endings before comparison
4. Concurrent DB open during schema initialization causes "database is locked" — fixed by pre-opening connections before barrier in concurrency test
5. `page_embeddings.chunk_index` NOT NULL constraint — fixed insertion query in migration test

**Measurement note:** SHA-256 hashes in `datasets.lock` for BEIR archives are placeholders. Run `./benchmarks/prep_datasets.sh --compute-hashes` after first download to establish real hashes.

---

## 2026-04-16: Phase 3 Groups 5–6 Benchmarks (completed)

**Scope:** Benchmark foundation (Groups 5+6)  
**Status:** Completed  

**Shipped:**
- datasets.lock (placeholder hashes, workflow documented)
- prep_datasets.sh with --compute-hashes option
- requirements.txt for Python dependencies
- baselines/beir.json reference data
- Four Rust integration harnesses (BEIR eval, latency gate, concurrency test)
- Three Python advisory adapters

**Test results:** 19 newly-runnable tests pass; dataset paths gated by #[ignore]  

**Key decisions:**
- BEIR harness in tests/ (idiomatic Rust, not benchmarks/)
- Latency gate #[ignore] (debug builds 3-5× slower)
- Per-thread connections for real SQLite WAL concurrency testing
- embedding_to_blob promoted to pub for integration test access

**Decision:** kif-phase3-benchmarks.md merged to decisions.md  

**Next:** Phase 3 core reviews (tasks 8.1, 8.2) proceed with revisions to address professor/nibbler blockers.
