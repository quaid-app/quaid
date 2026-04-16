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
