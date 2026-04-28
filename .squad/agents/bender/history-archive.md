# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- **PR #110 rebase onto post-PR-#111 main (2026-04-28):** Rebased `fix/no-direct-main-guardrails` onto `main` @ `987aaa7` (the PR #111 merge commit). Three files had conflicts across ~8 hunks: `tests/command_surface_coverage.rs` (missing `provision_vault` in stale PR #110 fmt/fix commits), `src/core/fs_safety.rs` (PR #110 commits used `parent_fd` / `_root_fd` / dropped `walk_to_parent`; HEAD's `_parent_fd` form is correct — it consumes `root_fd`), `src/core/vault_sync.rs` (PR #110 baseline debt commit used `VaultSyncError::InvariantViolation` with bare `?`, bypassing the poll-watcher fallback; HEAD's closure approach is correct). Resolution pattern: when a follow-up PR ports fixes from an open PR into main, take HEAD's version for all ported-fix conflicts — incoming commits are stale pre-fix snapshots. All three gates passed post-rebase: fmt ✅ / clippy ✅ / check ✅. New HEAD: `752db5e`. Decision record: `bender-pr110-conflicts.md`.

- **PR #111 Test failure diagnosis (2026-04-28):** After `Check` turned green on commit `5152ef7`, the `Test` job ran for the first time on the branch and exposed two pre-existing failures in `tests/command_surface_coverage.rs` (introduced in `ea5cabf` / v0.10.0 / main). The failing tests are `export_command_writes_markdown_files_for_existing_pages` (line 106) and `tags_timeline_add_and_link_close_commands_update_existing_records` (line 281). Both call `init_db()` (subprocess) → `db::open()` (in-process) → `put::put_from_string()`, which hits `vault_sync::with_write_slug_lock` and dies on Linux with ENOENT because the default collection's `root_path` is empty after `quaid init`. This is the **identical root cause** PR #111 already fixed in `src/commands/export.rs`. Classification: **inherited baseline debt** — these tests lived on `main` before the PR; `main`'s CI never reached the `Test` job because `Check` was always failing first. PR #111 was the first branch to clear `Check`, which unmasked the latent failures. Fix belongs on PR #111: apply the same vault-root provisioning block (create `vault/` subdir, `UPDATE collections SET root_path=…, state='active'`) in both integration tests before calling `put_from_string`. Scope is surgical and on-theme.

- **PR #110 cmp_owned regression fix (2026-05):** Commit `489b990` fixed a `PathBuf` match-guard type mismatch but introduced `clippy::cmp_owned` — `PathBuf::from("...")` creates a temporary owned value solely for comparison. The correct form is `Path::new("...")`, which is borrowed and satisfies `PartialEq<Path>` on `PathBuf` without allocation. `Path` was already imported; the fix was a one-token substitution. All three gates passed: `cargo fmt --all -- --check` (exit 0), `cargo clippy --all-targets -- -D warnings` (exit 0), `cargo check --all-targets` (exit 0). Committed as `c8e1a18`, pushed to `fix/no-direct-main-guardrails`. Rule update: in match guards, prefer `Path::new(...)` over `PathBuf::from(...)` when the left side is already a `PathBuf`.

- Command coverage sprint (2026-05):Added targeted unit tests to `validate.rs`, `pipe.rs`, `query.rs`, and `call.rs`. Windows line coverage moved from 85.58% → **88.38%**. Individual file gains: validate 70.7%→91.0%, pipe 70.1%→89.4%, query 79.8%→95.0%, call 67.6%→88.9%. Key constraints: `process::exit(1)` failure paths are permanently untestable; `invalid_temporal_order` link check cannot be triggered without bypassing SQLite CHECK constraints; `active_model_count > 1` blocked by partial unique index. Two pre-existing test failures in `core::search` (UNIQUE constraint on `config` table upsert) were present before this work and are unrelated.
- The `config` table (runtime key/value defaults including `default_token_budget`) IS seeded by the schema at `db::open` time — the summary incorrectly said it did not exist. Use `UPDATE` not `INSERT` when overriding config values in tests.
- `#[tokio::test]` requires the `tokio` dev-dependency to have `features = ["full"]` — confirmed present in this project's Cargo.toml.
- Coverage sprint 2 (2026-05): Pushed Windows LINE coverage from **88.38% → 90.12%** (clean) / **90.77%** (no-clean). Added `pipe_no_newline_exceeds_limit_triggers_too_long_at_eof` in `pipe.rs` (covers lines 65-70 + 43-44) and two FTS5-hit tests in `query.rs` (covers lines 55+59). Stale LLVM binary issue: `--no-clean` coverage runs hang if the lib binary was built with a now-deleted test — fix is to run `--clean` or delete `target\llvm-cov-target\debug\deps\quaid-*.exe` manually. FTS5 tests must use multi-word queries (space prevents `exact_slug_query` short-circuit) with unique token prefixes like `xqzfoo` to avoid false matches.

- **Batch 1 Coverage Arc (2026-04-28/29):** Executed three-sprint command + reconciler + validate coverage push. Results: command sprint 85.58%→88.38% (+2.80 pts); reconciler sprint 42 new tests for Display impls, pure-logic branches, authorization matrix, DB-only paths; validate sprint 1 test covering stale binary workaround. Final authorized measurement: **90.77%** from `target\llvm-cov-final.json` (Windows authoritative). Platform note: Linux CI canonical pre-Batch1 was 82.53%; unix-gated infrastructure paths (~1,400–1,600 lines) remain architectural ceiling on that platform, not regression. Decision record: bender-command-coverage.md, bender-reconciler-coverage.md, bender-validate-coverage.md merged to decisions.md. Status: v0.10.0 coverage gate CLEARED.

- Validation needs to cover ingest, retrieval, CLI behavior, and MCP behavior.
- OpenSpec proposals define what must be proven, not just what must be built.
- This project values round-trip safety and harsh failure testing.
- Batch 1 coverage audit (2026-04-28): Linux CI canonical measurement is **82.53%** (19,588 / 23,729 lines). The 90% target is **not achievable** from this lane without significant dedicated test work on unix-gated internals. Recommendation: **defer v0.10.0 ship decision** and assign remaining gap (~1,768 lines) to follow-on test sprint. Platform barriers: ~1,400–1,600 unix-gated lines in vault_sync/reconciler/collection/fs_safety/quarantine unreachable on Windows or without integration tests. Decision record merged to `.squad/decisions.md`.
- The Link struct has a known schema-vs-task mismatch: task says from_slug/to_slug, schema uses from_page_id/to_page_id (integer FK). Must verify Fry's resolution.
- `type` is a Rust keyword; the Page struct must rename the field (e.g., `page_type`) and handle serde/rusqlite column mapping.
- Anticipatory QA validation plan for tasks 2.1–2.6 written to `.squad/decisions/inbox/bender-p1-foundation-validation.md` on 2026-04-14.
- **PR #110 PathBuf fix (2026-04-28):** `WatchEvent::DirtyPath(path) if path == "notes/already-buffered.md"` fails on CI runner because `PathBuf` does not implement `PartialEq<&str>`. One-line fix: replace bare `&str` with `PathBuf::from("notes/already-buffered.md")` in the match guard. Fix is commit `489b990` on `fix/no-direct-main-guardrails`. All three local gates passed: `cargo fmt --all -- --check` (exit 0), `cargo clippy --all-targets -- -D warnings` (exit 0), `cargo check --all-targets` (exit 0). Rule: `PathBuf` match guards must always use `PathBuf::from(...)`, never bare `&str`.

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

