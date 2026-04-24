# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Validation needs to cover ingest, retrieval, CLI behavior, and MCP behavior.
- OpenSpec proposals define what must be proven, not just what must be built.
- This project values round-trip safety and harsh failure testing.
- Phase 1 tasks 2.1–2.6 (core types) are the foundation — schema-struct alignment is the highest-value check before any downstream work.
- The Link struct has a known schema-vs-task mismatch: task says from_slug/to_slug, schema uses from_page_id/to_page_id (integer FK). Must verify Fry's resolution.
- `type` is a Rust keyword; the Page struct must rename the field (e.g., `page_type`) and handle serde/rusqlite column mapping.
- Anticipatory QA validation plan for tasks 2.1–2.6 written to `.squad/decisions/inbox/bender-p1-foundation-validation.md` on 2026-04-14.

## 2026-04-14 Scribe Merge (2026-04-14T03:50:40Z)

- Orchestration logs written for Bender (validation plan) and other Phase 1 agents.
- Session log recorded to `.squad/log/2026-04-14T03-50-40Z-phase1-db-slice.md`.
- Bender's validation plan decision merged into `decisions.md` (anticipatory QA checklist for T02–T06).
- Three decisions merged total; inbox files deleted.
- Fry, Leela, Bender histories updated with cross-team context.
- Ready for git commit.


## 2026-04-14T04:07:24Z Phase 1 T03 Markdown Slice Approval

- Reviewed src/core/markdown.rs (commit 0ae8a46) against all spec invariants.
- Verdict: APPROVED. All 4 public functions match spec; 19/19 unit tests pass; no violations.
- Two non-blocking concerns documented for Phase 2:
  1. Naive YAML rendering loses structured values
  2. No lib.rs blocks integration tests (Phase 1 ship gate blocker)
- lib.rs gap flagged as Phase 1 blocker for integration test harness.

## 2026-04-14T04:07:24Z Scribe Merge (T05, T07, T03 approval, T06 spec)

- Scribe wrote 3 orchestration logs (Fry T05+T07, Bender T03 approval, Scruffy T06 spec).
- Scribe wrote session log for Phase 1 command slice window (3h execution).
- Four inbox decisions merged into canonical decisions.md (no duplicates found).
- Inbox files deleted after merge.
- Cross-agent history updates applied.
- Ready for git commit.

## 2026-04-14 Phase 1 Search/Embed/Query Validation

- Validated T14, T16, T18, T19 against implementation code. All 113 tests pass.
- **Finding 1:** `gbrain embed <SLUG>` (single-page embed) is not implemented. The clap CLI only has `--all` and `--stale` flags, no positional slug argument. T18 checkbox correctly open.
- **Finding 2:** `--token-budget` in `gbrain query` counts characters, not tokens. The flag name is misleading — a user passing `--token-budget 4000` gets ~4000 chars, not tokens. T19 spec says "hard cap on output chars in Phase 1" which is honest, but the flag name is a footgun.
- **Finding 3:** `embed()` in inference.rs is a deterministic SHA-256 shim, not Candle/BGE-small. Produces correct-shape vectors but no semantic similarity. BEIR benchmarks against this shim will be meaningless. T14 `[~]` status is honest but needs explicit documentation.
- No production code modified. Findings written to `.squad/decisions/inbox/bender-embed-validation.md`.
- No user-visible breakage found in current code — all paths that exist work correctly.
- Verdict: embed command is incomplete (missing single-slug mode), query budget semantics are misleading, inference shim status should be documented. All three must be addressed before Phase 1 ship gate.

## 2026-04-14T04:56:03Z Phase 1 Search/Embed/Query Closeout

- **Finding 1 (single-slug embed):** RESOLVED ✅ — Fry implemented `gbrain embed <SLUG>` support.
- **Finding 2 (token-budget flag):** ACCEPTED (Phase 1 design decision) — Flag name misleading but spec explicitly hard-caps to chars in Phase 1. Scoping rationale documented for Phase 2 flag rename when real tokenizer lands.
- **Finding 3 (inference shim status):** RESOLVED ✅ — Leela's revision cycle added explicit placeholder contract, stderr warnings, and honest task status notes. Module docs, runtime output, and task tracking all clarify: plumbing done, semantic deferred to Phase 2.
- **Validation coverage:** FTS5 (T13) contract ✅, embed command contract ✅, query command contract ✅, inference API shape ✅, integration paths ✅. All 115 tests pass. No production code breakage.
- **Orchestration log written:** `2026-04-14T04-56-03Z-bender-validation-closeout.md`
- **Outcome:** Phase 1 search/embed/query lane cleared for ship gate. All findings resolved or documented for Phase 2. Validation complete; clearance issued for Professor final approval.

## 2026-04-15 SG-7 Roundtrip Sign-off

- **Verdict:** CONDITIONAL APPROVE
- **roundtrip_semantic:** Validates normalized idempotency (import→export→reimport→export produces identical SHA-256 hashes). Tests page count AND content hashes — not superficial. Does NOT test source→export fidelity (YAML array tags silently dropped — known Phase 2 concern).
- **roundtrip_raw:** Validates byte-exact output against canonical inline fixture. Strongest possible assertion. Fixture is genuinely canonical.
- **Both tests pass.** Deterministic, zero flap risk.
- **Coverage gap:** No test checks that original fixture frontmatter keys survive import. Acceptable for Phase 1 since structured YAML support is Phase 2.
- **CI note:** `cargo test roundtrip` is a misleading filter — doesn't match integration test function names. Use `cargo test --test roundtrip_raw --test roundtrip_semantic` explicitly.
- SG-7 marked `[x]` in tasks.md. Decision written to `.squad/decisions/inbox/bender-sg7.md`.

## 2026-04-18 v0.9.0 Release Validation

- **Scope:** Validated real v0.9.0 release lane (tag, release workflow, npm publish workflow, release assets).
- **Release workflow (run 24516840337):** All 5 jobs green (4 platform builds + GitHub Release creation). All 8 binary+checksum artifacts uploaded. `install.sh` uploaded as 9th asset. Checksums re-verified post-download in CI. Linux binaries statically linked (verified).
- **Release assets:** 9 assets on GitHub Release page — 4 binaries (darwin-arm64, darwin-x86_64, linux-x86_64, linux-aarch64), 4 SHA-256 sidecars, 1 install.sh. All `uploaded` state. Not draft, not prerelease.
- **Publish npm workflow (run 24516842061):** Token-guard exercised in real CI. NPM_TOKEN absent → skip-notice printed → `npm pack --dry-run` validated (4 files, 2.4KB tarball, binary excluded) → publish step skipped. Workflow concluded `success`.
- **D.5 CLOSED ✅:** Token-guard behavior proven through real CI execution. Negative path (no token) confirmed. Positive path (token present) by-design deferred — structural guard verified.
- **D.2 STILL OPEN:** v0.9.0 release existing removes the "not a real release" blocker. Asset-name alignment verified (postinstall.js platform mapping matches all 4 release assets). Windows EBADPLATFORM + WSL no-Node still blocks end-to-end postinstall test. Needs macOS/Linux runner.
- **Binary size observation:** Release binaries are 7.7–9.5MB, not ~90MB as originally estimated. Proposal claimed 90MB; actual is ~10× smaller. Not a blocker but D.2 task text corrected.
- **Key paths:** `.github/workflows/release.yml` (release pipeline), `.github/workflows/publish-npm.yml` (npm publish with token guard), `scripts/install.sh` (shell installer), `packages/gbrain-npm/scripts/postinstall.js` (npm postinstall downloader).
- Decision written to `.squad/decisions/inbox/bender-v0-9-0-release-validation.md`.

## 2026-04-16T14:59:20Z Simplified-install v0.9.0 Release — Bender Completion

- **Task:** Validated v0.9.0 release and publish-npm workflow success. Closed D.5, kept D.2 open with honest assessment, updated task tracking and decision log
- **Changes:**
  1. Release validation — confirmed v0.9.0 tag, binaries, and checksums
  2. Publish-npm workflow — verified GitHub Actions publish workflow succeeded
  3. Decision D.5 closure — closed with CI evidence (publish-npm workflow run)
  4. Decision D.2 assessment — kept open with honest status on publish reliability
  5. Task tracking — updated tasks/history with validation results
- **Status:** ✅ COMPLETE. v0.9.0 release validated. Publish workflow confirmed working. Team decision log updated honestly.
- **Orchestration log:** `.squad/orchestration-log/2026-04-16T14-59-20Z-bender.md`
- **Orchestration log:** `.squad/orchestration-log/2026-04-16T14-59-20Z-bender.md`

## 2026-04-19 v0.9.1 Dual-Release D.1 Validation

- **Verdict:** REJECT — one high-severity defect blocks sign-off.
- **Defect #1 (HIGH):** Cargo.toml `default = ["bundled", "embedded-model"]` (per approved task A.4) makes source-build default AIRGAPPED. But 9+ documentation files claim `cargo build --release` = "online channel (default)." Every "Build from source" instruction is wrong. Root cause: A.4 changed the default AFTER the Phase C doc sweep. No reconciliation pass followed. README Quick Start even shows two commands that both produce the same airgapped binary — the online channel is never shown.
- **Defect #2 (LOW):** Task B.3 claims postinstall.js "handles `GBRAIN_CHANNEL=airgapped|online` overrides" but code has no GBRAIN_CHANNEL support. `now.md` also overclaims. Near-zero impact since design says npm = online only.
- **Passing checks:** `cargo fmt --check` ✅, `cargo check` ✅, `cargo test` (285+ pass, 0 fail) ✅, `npm pack --dry-run` (4 files, no binary) ✅, no `gbrain-slim-*` naming ✅, compile_error guard ✅, release manifest ✅, installer defaults ✅, version bump ✅, no base/large promises ✅.
- **Revision owners:** Defect #1 → Hermes (doc sweep). Defect #2 → Fry (implementation or task text fix).
- **Decision written:** `.squad/decisions/inbox/bender-dual-release-verdict.md`

## 2026-04-19 v0.9.1 Dual-Release D.1 Rereview

- **Verdict:** APPROVE ✅
- **Defect #1 (HIGH — docs claimed `cargo build --release` = online):** FIXED. Hermes swept all 14+ doc surfaces. Every "Build from source" block and install table now correctly identifies `cargo build --release` as the **airgapped** channel (default). Explicit online flag `--no-default-features --features bundled,online-model` shown in all code blocks. No surface claims online is the source-build default.
- **Release contract verified coherent:**
  - `Cargo.toml` default = `["bundled", "embedded-model"]` → source-build = airgapped ✅
  - `scripts/install.sh` defaults to `GBRAIN_CHANNEL=airgapped` ✅
  - `postinstall.js` hardcodes `*-online` assets → npm = online ✅
  - `release.yml` matrix: 4 platforms × 2 channels = 8 binaries + 8 SHA-256 sidecars ✅
  - Channel names `airgapped`/`online` only — no `gbrain-slim-*` references in code/scripts/docs ✅
  - `compile_error!` guard present in inference.rs ✅
  - Version surfaces at `0.9.1` ✅
- **Non-blocking nits (not re-tested, carried from D.1 round 1):**
  1. `website/reference/spec.md:2249` uses "slim binary" as descriptive English for online-model. D.0 explicitly exempts descriptive prose. Cosmetic only.
  2. Defect #2 (LOW) — B.3 task text and `now.md` still claim `postinstall.js` supports `GBRAIN_CHANNEL` env override, but code doesn't implement it. Near-zero impact (npm = online only). Assigned to Fry; not in Hermes's revision scope.
- **Outcome:** The rejected defect is fixed. Release contract is coherent. Cleared for D.2 (PR + ship).

## Learnings

- **Always validate the task execution ORDER against doc accuracy.** When a task changes a default (A.4) after docs have been normalized (Phase C), the docs become stale. The right check is: "does the doc claim match what `cargo build --release` actually produces?" Not: "did someone mark the doc task done?"
- **Grep the actual `Cargo.toml` `[features] default` line and compare it to every doc that says 'default' near a channel name.** This single check would have caught this defect at the C.1 review stage.
- **postinstall.js env-var overrides should be explicitly tested or explicitly removed from the task spec.** A claimed-but-missing override is worse than no override — it makes the task look done when it isn't.
- **Rereviews should be scoped tightly.** When you rejected for one defect, the rereview checks that defect first, then a quick pass on overall coherence. Don't re-litigate low-severity findings that were assigned elsewhere.

## Session Completion: Dual Release v0.9.1 (2026-04-19)

**Status:** ✅ Session logged and decisions merged.

This dual-release cycle validated the full team workflow:
- OpenSpec proposal as source of truth ✓
- Implementation phase with clear gate criteria ✓
- Docs validation identifying defects early ✓
- Revision cycle to fix defects ✓
- Second validation round confirming all fixes ✓
- PR opened ready for merge ✓

**Lesson learned:** Implementation task ordering matters. When task A.4 changes a fundamental default (Cargo feature flags), document changes that happened before A.4 execution must be invalidated and re-checked after A.4 lands. There's no automatic re-trigger. This needs explicit mention in the pre-review checklist: "if any implementation task changed defaults, re-validate all public docs that mention that default."

## 2026-04-19 PR #47 Blocker Review

- **Verdict:** BLOCKED — three high-severity blockers remain unfixed.
- **Context:** Validated PR #47 (feat: configurable embedding model) against Professor and Nibbler review findings. Fry has addressed many review comments (`83a7c67`, `71666d7`, `d75bc0b` fixes), but the three core concurrency/integrity blockers are NOT YET RESOLVED.
- **Blocker 1 (UNFIXED):** Active-model registry transition uses two autocommit statements (`src/core/db.rs:188-204`), creating a zero-active-row gap visible to concurrent readers. **Fix:** wrap in transaction like `write_brain_config`.
- **Blocker 2 (UNFIXED):** Online downloads use fixed temp file names (`config.json.download`, etc.) in shared cache dir (`src/core/inference.rs:667`), causing clobber risk on concurrent first-load. **Fix:** unique temp names or per-model download lock.
- **Blocker 3 (UNFIXED):** CI online-model test job (`.github/workflows/ci.yml:71`) does not set `GBRAIN_FORCE_HASH_SHIM=1`, allowing flaky network-dependent tests. **Fix:** add env var to CI job.
- **Key files examined:** `src/core/db.rs` (registry flip), `src/core/inference.rs` (download + ensure_model), `.github/workflows/ci.yml` (test jobs).
- **Evidence:** `write_brain_config` was correctly fixed with a transaction (lines 229-250), confirming Fry knows the pattern. Registry flip and download paths were not updated.
- **Decision written:** `.squad/decisions/inbox/bender-pr47-validation.md` — includes validation plan for post-fix execution.
- **Recommendation to Fry:** Fix all three blockers in order (atomic registry → hermetic CI → download safety), then ping for re-validation.

## Learnings

- **Review bot comments vs. human review decisions:** PR #47 had 19 bot review threads, but only 3 were called out as merge-blocking by Professor and Nibbler. Always read the explicit human review decisions (`.squad/decisions/inbox/*-pr*-review.md`) to know what actually blocks merge vs. what's noise/nice-to-have.
- **Atomic DB state transitions require explicit transactions.** Even when one function (`write_brain_config`) uses a transaction correctly, a related function (`ensure_embedding_model_registry`) may still use autocommit. Both must be checked independently.
- **Concurrent download safety is subtle.** Fixed temp file names look safe in single-process use but fail under concurrent load. Validation must explicitly think "what if two threads call this at once?"
- **CI hermetic testing requires global env vars, not per-test guards.** A test setting `GBRAIN_FORCE_HASH_SHIM=1` via `EnvVarGuard` does NOT make the entire CI job hermetic — the CI job itself must set the env var.
- **Fix velocity ≠ fix correctness.** Fry produced 4+ fix commits in rapid succession, addressing many bot comments. But the three blockers (which require deeper concurrency reasoning) were not addressed. Volume of fixes is not the same as blocker closure.
**Lesson learned:** Implementation task ordering matters. When task A.4 changes a fundamental default (Cargo feature flags), document changes that happened before A.4 execution must be invalidated and re-checked after A.4 lands. There's no automatic re-trigger. This needs explicit mention in the pre-review checklist: "if any implementation task changed defaults, re-validate all public docs that mention that default."

## 2026-04-24 M1b-i Write-Gate Proof Closure

- **Scope:** 17.5s2–17.5s5 — write-interlock proof batch (tests/evidence only, no production changes).
- **Finding:** No missing behavior. All five MCP mutators (`brain_put`, `brain_link`, `brain_check`, `brain_raw`, `brain_gap` slug-bound) already call `ensure_collection_write_allowed` before any mutation. The gate is consistently wired under task 11.8.
- **New tests (6):** `brain_put_refuses_when_collection_is_restoring`, `brain_link_refuses_when_collection_is_restoring`, `brain_link_refuses_when_collection_needs_full_sync`, `brain_check_refuses_when_collection_is_restoring`, `brain_check_refuses_when_collection_needs_full_sync`, `brain_raw_refuses_when_collection_is_restoring`. All 6 pass.
- **Pre-existing tests that already covered remaining matrix cells (5):** `brain_put`/`brain_raw` + `needs_full_sync`, `brain_gap` slug ×2, `brain_gap` slug-less.
- **17.5s2 mutator matrix:** 5 mutators × 2 conditions = 10 cells, all proved. `brain_gap` slug-less correctly excluded (Read carve-out).
- **Tasks closed:** 17.5s2 ✅, 17.5s3 ✅, 17.5s4 ✅, 17.5s5 ✅
- **Lesson:** When auditing write-gate coverage, build the explicit (mutator × condition) matrix first. Ad-hoc sampling by condition or by op alone will miss cells.

## 2026-04-24 M1b-ii/M1b-i Session Completion

- **M1b-i proof lane COMPLETE:** Write-gate restoring-state proof closure (tests-only). Found no missing behavior. All mutators already call `ensure_collection_write_allowed` before mutation.
- **M1b-ii implementation lane COMPLETE (Fry):** Unix precondition/CAS hardening. Real `check_fs_precondition()` helper with self-heal; separate no-side-effect pre-sentinel variant for write path.
- **Inbox decisions merged:** Bender M1b-i write-gate closure + Fry M1b-ii precondition split decision. Both entries now in canonical `decisions.md`.
- **Orchestration logs written:** `2026-04-24T12-53-00Z-bender-m1b-i-proof-lane.md` + `2026-04-24T12-54-00Z-fry-m1b-ii-implementation-lane.md`.
- **Session log written:** `2026-04-24T12-55-00Z-m1b-session.md`.
- **Status:** Awaiting final Professor + Nibbler gate approval for both M1b-i and M1b-ii.

## Learnings

- **Exact-slug shortcuts must fail closed before generic search fallback.** If a hybrid-query path recognizes a bare slug or `[[slug]]`, ambiguity is a routing failure, not a "no results" case. Returning `None` from the exact-slug fast path silently lies about the seam and hides duplicate-slug defects.
- **For CLI parity claims, prove every slug-bearing entry point directly.** One `get` ambiguity test is not evidence for `graph`, `timeline`, `check`, `link`, `links`, `backlinks`, `unlink`, or exact-slug `query`. Build the command matrix first, then add one direct refusal assertion per command family so the task text stays truthful.
- **For frozen MCP diagnostic schemas, test the full predicate, not just the label column.** A terminal discriminator like `integrity_blocked` must prove its timestamp/age gate, precedence, and negative cases (reason present without terminal state, queued recovery, pre-window restore) or reviewers will correctly reject it as overclaimed.

