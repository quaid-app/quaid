# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Vault-sync watcher/runtime coverage (2026-04-28): the highest-value uncovered watcher branches were the ones only reachable through private runtime seams, not the already source-locked production-shape tests. Direct unit proofs for crashed-watcher backoff hold, session-scoped watcher-health publishing, overflow-recovery lease-mismatch refusal, and watch-callback channel full/closed behavior add real branch depth in `src/core/vault_sync.rs`, but this lane alone cannot move a repo-wide 84.51% line gate to >90%—the denominator is ~24,136 lines, so the remaining gap is roughly 1,326 covered lines.
- Batch 1 watcher coverage decision (2026-04-28): treat the watcher-runtime branch-depth repair as a **watcher-runtime branch-depth repair, not as a path to 90% repo-wide gate**. Coverage gate remains unresolved. Batch 1 release decision deferred to broader team evaluation (Bender audit, Mom continuation lanes running parallel). Scruffy decision: watcher lane validated by passing tests and source/branch evidence; repo-wide release gate blocked on broader work or repaired coverage environment.
- Quarantine-restore proof finalize (2026-04-25): Fry's Unix restore seam is real now, so the honest proof lane graduates from ignored scaffolding to deterministic behavior tests. Scruffy's closure in `.squad/orchestration-log/2026-04-25T15-48-57Z-scruffy.md` marks the lane as closed narrowly (no longer scaffolding). All accessible tests pass (591 total, plus 5 Unix-specific pending Linux CI).
- Quarantine-restore proof lane (2026-04-25): with `collection quarantine restore` now live on Unix, the honest next move is to exercise both blocker seams with deterministic hooks: install-time no-replace race injection and post-install rollback proving parent `fsync` after every successful unlink.
- Quarantine-restore proof lane (2026-04-25): with `collection quarantine restore` still hard-deferred in `src/commands/collection.rs`, the honest next move is ignored proof scaffolding, not pretend-green behavior tests. I parked one ignored CLI happy-path proof for `17.5j` in `tests/collection_cli_truth.rs` plus two ignored blocker proofs in `tests/quarantine_revision_fixes.rs` that spell out the exact deterministic seams Fry must expose next: install-time no-replace race injection and post-install rollback proving parent `fsync` after every successful unlink.
- Vault-sync watcher/quarantine gap audit (2026-04-25): landed two quarantine epoch-regression proofs in `src/core/quarantine.rs` and one watcher lifecycle source-invariant proof in `src/core/vault_sync.rs`. The sharp remaining seams after this batch are still (a) a direct proof that every hard-delete path consults the DB-only-state guard (`reconciler` missing-file delete, quarantine discard, TTL sweep) and (b) the live watcher overflow/full-sync fallback path when the bounded notify channel fills; quarantine restore remains explicitly deferred and should not be reopened by test wording.
- Vault-sync coverage audit (2026-04-24): the sharpest remaining branch value is split between Unix-only watcher internals in `src/core/vault_sync.rs` and quarantine CLI truth in `tests/quarantine_revision_fixes.rs` / `tests/collection_cli_truth.rs`. The most meaningful missing proofs are watcher queue-overflow / watcher-reload replacement behavior plus quarantine list/export/discard epoch + payload completeness; `tests/watcher_core.rs` stays cfg-gated on this Windows host, so deeper watcher branch tests should live beside the private watcher helpers in `src/core/vault_sync.rs`.
- Vault-sync-engine post-watcher-core proof lane (2026-04-25): the truthful narrow batch is already split in two. `src/core/vault_sync.rs` now contains the right Unix-only direct proofs for `7.5` (`writer_side_rename_failure_cleans_tempfile_dedup_and_sentinel_without_touching_target`, `writer_side_post_rename_fsync_abort_retains_sentinel_removes_dedup_and_marks_full_sync`), while quarantine lifecycle tasks `17.5g7/17.5h/17.5i/17.5j` still have no landed `collection quarantine` seam to exercise and `collection info` still does not surface quarantine-awaiting counts. On this Windows host I could re-run the collection-status proofs, but the `7.5` tests stay cfg-gated rather than executable; approve the dedup-cleanup proof lane on code inspection/scope, and keep quarantine/export/discard/restore/TTL claims explicitly deferred until Fry lands operator-facing production hooks.
- Vault-sync-engine watcher-core next slice (2026-04-25): after `17.5aa5`, the honest reviewer-facing proof lane is still narrower than the spec wishlist. `notify`/debounce/path+hash TTL dedup are not present in production yet (`Cargo.toml` still lacks `notify`; task 6.x/7.x remain open), so the only public seam I could truthfully lock was `start_serve_runtime()` deferring a fresh restore heartbeat without mutating `pages`/`file_state`/`raw_imports` or enqueuing embeddings. Treat debounce batching, TTL suppression, expiry acceptance, and path-only non-suppression as blocked on Fry landing a testable watcher/dedup surface rather than something tests should speculate into existence.
- Vault-sync-engine 13.6 + 17.5ddd proof map (2026-04-24): treat `brain_collections` as a frozen-schema proof slice, not a generic "collection list exists" claim. The gate must assert the exact 13-field keyset plus exact nullability/tagged-union behavior: `root_path` masked to null when `state != 'active'`; `ignore_parse_errors` uses only the canonical array-of-objects shape (`parse_error` with populated `line`/`raw`, `file_stably_absent_but_clear_not_confirmed` with both null); `needs_full_sync`/`recovery_in_progress` encode queued vs running recovery without inventing extra fields; `integrity_blocked` uses the exact discriminator strings and precedence; `restore_in_progress` is distinct from plain `state='restoring'`. The sharpest under-proved seam is trusting existing CLI `collection info/list` output or status helpers: they expose extra fields, stringify writable differently, currently use non-frozen blocker values like `manifest_incomplete_pending`, and do not by themselves prove the MCP tool is read-only or schema-exact.
- Vault-sync-engine 13.3 proof map (2026-04-24): the honest CLI parity gate is command-surface proof, not helper-only proof. Each slug-bearing CLI command needs its own direct subprocess coverage for explicit `<collection>::<slug>` success and ambiguous bare-slug refusal, while page-referencing output commands need separate assertions that they emit canonical `<collection>::<slug>` strings rather than raw `p.slug` values. The sharpest hidden traps are `check` (it pre-resolves the slug, then passes the canonical string into assertion helpers that still query `pages.slug` bare) and `graph` (it still roots and serializes on bare slugs through `core::graph`), so those two cannot be waved through on shared-helper confidence alone.
- Vault-sync-engine Batch L1 proof lane (2026-04-23): **APPROVED FOR LANDING**. All 11 mandatory proofs from Nibbler gate addressed and documented. The most credible startup proofs live in `src/core/vault_sync.rs` itself—pin the 15-second heartbeat gate directly, then exercise `start_serve_runtime()` with real stale-owner residue so the test can prove exact-once orphan finalize, `collection_owners` winning over ambient `serve_sessions`, and no supervisor-ack residue on the restore-only lane. Keep the claim narrow: these proofs certify restore-orphan startup recovery, not generic `needs_full_sync` or remap healing. Scope guardrail maintained: proof lane covers restore-owned pending-finalize state only, does NOT support generic needs_full_sync, remap attach, sentinel recovery, or broader "serve heals dirty collections" claims. **Batch L1 proof lane APPROVED FOR LANDING.**
- Vault-sync-engine Batch K1 re-gate (2026-04-23): after Leela's repair, the honest K1 surface is now `1.1b`, `1.1c`, `9.2`, `9.2b`, `9.3`, `17.5qq10`, and `17.5qq11`—because `brain_gap` now returns `page_id`, the read-only gate is explicitly scoped to vault-byte writers only, and both CLI `put` plus MCP `brain_put` are directly refusal-tested. Default `cargo test --quiet` passed; the Windows online-model lane still crashes while compiling dependencies with allocator/backtrace noise, which looks environmental rather than caused by the K1 slice.
- Vault-sync-engine Batch K2 proof lane (2026-04-23): the honest K2 proofs landed for `17.5kk3`, `17.5ll3`, `17.5ll4`, `17.5ll5`, and `17.5mm` once offline restore stopped bypassing `finalize_pending_restore()`, retryable manifest gaps pointed operators back to `sync --finalize-pending`, and `restore-reset` was restricted to terminal integrity failure. `17.11` remains unclaimed because offline completion still needs the serve/RCRT attach path rather than a pure CLI finalize path.
- Vault-sync-engine Batch M1a writer-side sentinel crash core (2026-04-24): keep the proof lane narrow and internal. The honest seam is a Unix-only crash-core helper in `src/core/vault_sync.rs` plus direct tests that pin sentinel-create failure, pre-rename cleanup, rename cleanup, post-rename fsync abort retention, and foreign-rename + `SQLITE_BUSY` sentinel-only recovery into startup reconcile. Do not claim CLI/MCP routing, mutex proof, happy-path commit closure, live worker healing, or generic startup repair from this slice alone.
- Vault-sync-engine Batch L1 proof lane (2026-04-23): the most credible startup proofs live in `src/core/vault_sync.rs` itself—pin the 15-second heartbeat gate directly, then exercise `start_serve_runtime()` with real stale-owner residue so the test can prove exact-once orphan finalize, `collection_owners` winning over ambient `serve_sessions`, and no supervisor-ack residue on the restore-only lane. Keep the claim narrow: these proofs certify restore-orphan startup recovery, not generic `needs_full_sync` or remap healing.
- Vault-sync-engine Batch K1 (2026-04-23): proof stays honest when we separate "attach/read-only/list" from the wider mutator matrix. I tightened direct proofs for slug-less vs slug-bound `brain_gap`, CLI `collection info/list` read-only truth, and CLI `put` refusing persisted read-only collections; but the broader `CollectionReadOnlyError` rollout is still not credible for `check`, `link`, `tags`, `timeline`, or MCP write paths because they still route through the restoring-only gate.
- Vault-sync-engine Batch J (2026-04-23): **PROOF LANE COMPLETE, VALIDATION PASSED**. Narrowed batch proof lane strengthened all 15 test cases in `tests/collection_cli_truth.rs` covering active-root reconcile path, all five blocked states (fail-closed gates), duplicate/trivial halt terminal behavior, lease acquire/heartbeat/release (panic-safe via RAII), and operator diagnostics on CLI `collection info --json`. All tests pass in default lane ✅ and online-model lane ✅. Scruffy decision: narrowed batch supported; CLI-only truthfulness; MCP deferred. All seven IDs + two proofs credible.
- Batch H coverage is best anchored at `authorize_full_hash_reconcile()` and `hash_refusal_reason()` while the restore/remap pipeline is still forming: active tests can pin caller-identity gating and empty/trivial-body refusals now, and ignored seam tests should carry the exact blockers for phase ordering, 64-byte canonical reuse, and attach-completion write-gate sequencing until those helpers exist.
- Batch G raw_imports repair is safest when `apply_reingest()` is pinned directly on both existing-page seams: explicit `existing_page_id` and slug-matched lookup must refuse zero-total history before any page/file_state mutation, while truly new pages may still bootstrap their first raw import row.
- Batch G coverage stays honest when active tests pin the implemented reconcile/put boundaries (unchanged=no rotation, changed=rotation, stored UUID preserved) and deferred `full_hash_reconcile` / render-backfill behavior is locked behind ignored seam tests with exact task blockers.
- Batch F coverage is most truthful when current-idempotency assertions stay live while raw_imports/apply invariants are locked as ignored seam tests with exact task blockers; otherwise the suite either blesses missing safety work or goes red before implementation exists.

- The team wants high unit-test coverage, not token test presence.
- Proposal-first work helps define the invariants tests must guard.
- Coverage depth is a first-class role in this squad.
- Batch D coverage is strongest when `has_db_only_state` exposes each safety branch directly; otherwise a single SQL OR hides which quarantine guard regressed.
- Source-truth tests matter at audit seams: programmatic links should be written explicitly as `source_kind='programmatic'`, and imported assertions should stay `asserted_by='import'` so the delete classifier does not quarantine vault-derivable state by mistake.
- Reconciler symlink safety needs boundary tests, not just primitive tests: root rejection, skipped symlink entries, and repeat walks all belong on the reconciler seam itself.
- Vault-sync adds large new stateful surfaces (watchers, reconciliation, restore/finalize, collection routing) — must track separately from repo legacy backfill.
- Coverage denominator ambiguity (src only? all Rust? which features? which platforms?) blocks hard gate enforcement — scope must be explicit first.
- Foundation-slice checkmarks are not credible until schema-compatible legacy tests are repaired; otherwise coverage numbers on the new seam are meaningless.
- When a foundation slice has 181 test failures post-PR, the issue is likely NOT coverage but schema/write-path coherence. Coverage metrics are only meaningful after the foundation is stable.
- A repaired foundation can still be under-tested: green default+online suites do not prove collection-routing logic, schema-version refusal, or quarantine filters unless those branches have direct assertions.
- Once the foundation’s three gating seams each have direct tests (`parse_slug`, quarantined `search_vec`, pre-v5 refusal-before-DDL), green default+online suites are strong enough to approve the slice and carry the remaining matrix debt into the next batch.
- The carry-forward `parse_slug()` debt was real: single-collection shortcut, read unique/not-found, write-create without a write-target, and update/admin multi-owner ambiguity each needed their own direct assertions rather than relying on adjacent matrix cases.
- Ignore-pattern coverage is more credible when it locks the error payload shape, not just success/failure: raw failing lines, canonical `code`, stable-absence sentinel shape, and parse-error clearing on the next valid reload are all part of the contract.
- `file_state` helper coverage can defend the reconciler seam before the full engine lands: ctime-only and inode-only drift must trigger re-hash even when mtime and size stay stable, and timestamp fields should be asserted at the row-helper layer.
- Wrapper seams need their own tests, not just primitive coverage: `stat_file_fd()` should directly prove it preserves nofollow behavior and full stat population, even if `fs_safety::stat_at_nofollow()` already has syscall-level tests.
- Reconciler foundations need explicit non-destructive contract tests while still stubbed: `full_hash_reconcile()` must stay empty-success, and pre-walk `stat_diff()` should loudly show DB rows as missing rather than pretending discovery happened.
- A repaired stub slice is approvable once the safety-critical defaults fail loudly instead of succeeding quietly, direct tests pin those error messages/contracts, and task notes still say the real walk/hash behavior is deferred.
- Batch E frontmatter coverage is strongest at round-trip seams: lock `gbrain_id` through parse/render, import/export, and serde before UUID columns are fully wired.
- When UUID adoption is only partially implemented, avoid tests that bless the placeholder state; cover source-byte non-rewrite and explicit quarantine outcomes instead, then call out the exact missing seam.
- Batch E rename-guard coverage needs a direct adversarial template seam: large frontmatter plus a tiny non-empty body must still refuse hash pairing, while long-body hash/UUID positives stay named around what actually succeeds.
- Batch J proof is most credible when plain sync is pinned at both layers: `vault_sync::sync_collection()` must prove active-root reconcile, short-lived lease lifetime, and terminal duplicate/trivial halts, while CLI tests must prove blocked-state diagnostics and fail-closed operator messaging stay truthful.

## 2026-04-24 Vault-Sync 13.6 + 17.5ddd Slice Review

- Verdict: **REJECT**
- Smallest blocker: `src/core/vault_sync.rs::parse_ignore_parse_errors()` filters `ignore_parse_errors` down to `code == "parse_error"` only, while the frozen `brain_collections` schema in `openspec\changes\vault-sync-engine\design.md` still says the field's canonical tagged union includes both `parse_error` and `file_stably_absent_but_clear_not_confirmed`.
- Proof of mismatch: `src/mcp/server.rs::brain_collections_surfaces_status_flags_and_terminal_precedence()` explicitly locks the out-of-spec behavior with `absent["ignore_parse_errors"].is_null()`, so the current slice does not truthfully satisfy task `17.5ddd` ("response shape matches design.md schema exactly") even though the other narrowed status-field proofs look sound.

## 2026-04-24 Vault-Sync 13.6 + 17.5ddd Slice Re-review

- Verdict: **APPROVE**
- The spec seam is now coherent: `design.md` and `tasks.md` explicitly limit task `13.6` to line-level `parse_error` surfacing and defer the stable-absence refusal arm to `17.5aa5`, which matches the existing read-only `brain_collections` implementation and tests in `src\core\vault_sync.rs` and `src\mcp\server.rs` without overclaiming anything else inside `13.6 + 17.5ddd`.

## 2026-04-22 Vault-Sync Foundation Third Gate — Approved With Explicit Debt

**Session:** Scruffy re-gated Professor's three required coverage seams after the third-author repair pass.

**Validation rerun:**
- `cargo test --quiet` passed on the default channel.
- `GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model` passed on the online channel.

**What changed enough to clear the gate:**
- `src/core/collections.rs` now has direct `parse_slug()` tests for explicit addressing, multi-owner ambiguity, write-target create routing, write-create conflict, write-update miss, and write-admin single-owner resolution.
- `src/core/inference.rs` now directly asserts quarantined pages are excluded from `search_vec()` even when embedding rows exist.
- `src/core/db.rs` now directly asserts v4 databases are refused before v5 tables are created, for both `open_with_model()` and `init()`.

**Gate call:** APPROVE for the next implementation batch. The repaired foundation is compatibility-safe enough to build on, and the previously missing branch tests are now present where they mattered.

**Carry-forward debt for the next slice:**
- Expand `parse_slug()` coverage to the remaining matrix edges (single-collection bare-slug shortcut, read single-owner/not-found, write-create without write-target, multi-owner ambiguity for update/admin).
- Add a touched-surface coverage threshold once denominator scope is locked; current approval is based on seam depth plus green dual-channel suites, not a hard numeric coverage gate.

## 2026-04-22 Vault-Sync Foundation Re-review — Compatibility Repaired, Branch Depth Still Thin

**Session:** Re-checked Leela's repaired foundation slice strictly from compatibility and unit-test depth.

**Validation rerun:**
- `cargo test --quiet` passed on the default channel.
- `cargo test --quiet --no-default-features --features bundled,online-model` passed on the online channel with `GBRAIN_FORCE_HASH_SHIM=1`.

**Assessment:**
- Compatibility repairs look real: legacy ingest/import paths still have `ingest_log`, legacy page inserts route through `collection_id DEFAULT 1`, and composite upserts were repaired to `ON CONFLICT(collection_id, slug)`.
- The slice is now stable enough to build on, but the newly introduced collection-routing seam is not deeply tested yet.
- `src/core/collections.rs` currently has validator-only tests; the `parse_slug()` operation matrix (read vs create vs update/admin, single vs multi-collection, write-target vs ambiguity) still lacks direct coverage.
- The new quarantine-safety branches also lack direct regression tests: no targeted assertion for `search_vec()` excluding quarantined pages, and no targeted assertion for the explicit v4/v5 schema-version refusal path.

**Gate call:** REJECT for next implementation batch until the branchy foundation seams above get direct tests. Green suites prove the repairs stopped the bleeding; they do not yet prove the new routing and quarantine invariants will survive refactors.

## 2026-04-22 Vault-Sync Foundation Coverage — Coverage Metrics Only Credible After Coherence

**Session:** Scruffy reviewed vault-sync foundation slice for test coverage credibility. Foundation slice was concurrently rejected by Professor with 181 test failures.

**What happened:**
- Initial coverage assessment: new collections module achieved high branch coverage on added code
- Then Professor's review triggered: 181 `cargo test` failures due to schema NOT NULL constraints not wired into legacy INSERT sites
- Problem: coverage metrics on a broken foundation are misleading — they measure "branches exercised" not "branches correct"

**Lesson learned:**
- Foundation slices must have stable `cargo test` passing BEFORE coverage metrics become meaningful
- A 90%+ coverage number on broken code is worse than useless — it creates false confidence
- Wait for Leela's repair (181 failures → 0 failures) before re-assessing coverage

**Coverage recommendation for PR A (vault-sync-engine foundation):**
- Gate: `cargo llvm-cov --fail-under-lines 90` (configurable denominator: src only, default features only)
- This gate runs ONLY after `cargo test` passes cleanly
- Don't measure coverage on a branch that fails basic test execution

**Status:** Coverage assessment deferred until foundation repair is validated. Scruffy's coverage metrics will be re-run once schema is coherent.

## 2026-04-22 Vault-Sync-Engine Coverage Assessment

**Session:** macro88 directed team to treat `openspec\changes\vault-sync-engine` as next major enhancement with >90% test coverage.

**Coverage baseline audit:** Current `cargo llvm-cov report` shows `src/**` at **88.71% line coverage**. CI job is informational only (no enforced threshold). Biggest legacy sinks: `src/main.rs`, `src/commands/call.rs`, `src/commands/timeline.rs`, `src/commands/query.rs`, `src/commands/skills.rs`.

**Vault-sync surfaces:** New stateful surfaces (watchers, reconciliation, restore/finalize, write-through recovery, collection routing) can achieve ≥90% line coverage on their seams (unit + deterministic integration).

**Coverage denominator ambiguity — FLAGGED:** User requirement ">90% overall" is undefined in 3 dimensions:
- **Denominator:** `src` only vs all Rust including tests?
- **Feature scope:** default only vs default + online-model channels?
- **OS scope:** Ubuntu-only (current CI) vs unsupported Windows paths (`#[cfg(unix)]` fd-relative syscalls)?

**Repo-wide gate cost:** Promising >90% without legacy backfill would force unrelated cleanup (CLI orchestration files ≈11% coverage). Requires explicit backfill scope or denominator restriction.

**Two-tier coverage approach recommended:**
1. **Tier 1 (per-PR for new/touched vault-sync surfaces):** ≥90% line coverage at seam (unit + deterministic integration).
2. **Tier 2 (repo-wide reporting):** Continue informational coverage reporting. Do NOT promise hard repo-wide gate unless explicit scope decision: backfill work (0.5–1 day) OR denominator restriction (src only? default features only?).

**CI gate recommendation:** Add `cargo llvm-cov --fail-under-lines 90` hard gate in PR A (configurable denominator per scope decision).

**Decision:** Treat >90% overall as ambiguous until scope explicitly defined. Recommend two-tier approach with per-PR seam coverage (≥90%) and deferred repo-wide scope decision.

**Artifact:** `.squad/decisions/inbox/scruffy-vault-sync-coverage.md` (24 lines, flags ambiguity with practical recommendations)

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

## 2026-04-22: Vault-Sync Batch B Coverage Seams Locked

**What:** Completed targeted coverage work on vault-sync Batch B seams before full reconciler lands. Locked parse_slug routing matrix, .gbrainignore error-shape contracts, and file_state drift/upsert behavior.

**Decisions:**

### Early Seam Coverage Prevents Silent Refactor Failures
Helper-level tests as integration scaffold. Tests serve double duty: immediate validation of parse/ignore/stat helpers AND early warning system for integration hazards.

### Coverage Delivered
- parse_slug() routing matrix: all branching cases covered
- .gbrainignore error-shape contracts: all error codes and line-level reporting fidelity proved
- file_state stat-diff behavior: ctime/inode-only and mtime/size changes both trigger re-hash

**Validation:** 10 new direct unit tests for coverage seams. All tests pass. Error paths tested and will fail loudly if later changes break contracts.

**Why:** These are touched-surface seams with branchy behavior that future reconciler/watcher work will reuse directly. Guarding them now keeps Batch B credible even before the larger integration paths exist.

**Status:** ✅ COMPLETE. Ready for full reconciler implementation.

## 2026-04-22 Vault Sync Batch C — Re-gate (Approved)

**Session:** Scruffy coverage validation after Leela's targeted repair pass.

**What happened:**
- Scruffy re-reviewed the repaired Batch C to validate that foundation seams are locked with direct tests, safety-critical stubs explicitly error, and task claims are truthful.
- Focused on three seams: ile_state::stat_file_fd() (wrapper layer), econciler stubs (error contracts), 	asks.md (truthfulness).

**Key findings:**
1. **Direct seam coverage locked:** stat_file_fd() directly tested for nofollow preservation and full Unix stat field population. ull_hash_reconcile() and has_db_only_state() directly tested for explicit Err return. stat_diff() foundation behavior (DB rows as "missing") pinned by direct assertion.
2. **Safety-critical stubs explicitly error:** No more silent success defaults. econcile(), ull_hash_reconcile(), has_db_only_state() all required to return Err("not yet implemented") rather than Ok(empty).
3. **Task surface truthful:** Unchecked items remain pending; checked items annotated as foundation/scaffold. Deferred walk/hash/apply behavior not claimed complete.

**Validation rerun:**
- cargo test --quiet ✅
- GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model ✅

**Outcome:** APPROVE. Coverage sufficient on touched surface. Safety-critical stub behavior asserted directly. Ready to land.


### 2026-04-22 17:02:27 - Vault-Sync Batch E Coverage Lane

**Session:** Lock honest Batch E test coverage on real seams

**Coverage strategy:**

Do not add tests that would accidentally bless incomplete implementation or imply finished behavior. Focus on gbrain_id round-trip fidelity and ingest safety.

**Tests added (and locked):**

1. **gbrain_id frontmatter round-trip:**
   - parse_frontmatter() preserves gbrain_id
   - render_page() re-emits when present
   - import/export round-trip fidelity
   - serde serialization preserves the field

2. **Ingest non-rewrite behavior:**
   - Default ingest does not modify source markdown
   - Generated UUIDs stored in DB only, not in file
   - Git worktree stays clean after import

3. **Explicit delete-vs-quarantine outcomes:**
   - Quarantine classification on ambiguous/trivial cases
   - Delete predicate respects source_kind boundaries

**Tests explicitly NOT added:**

- Rename inference (native events, UUID matching, hash pairing) — these are deferred to Batch F apply pipeline
- Frontmatter write-back (brain_put UUID preservation) — deferred to later batch
- Watcher-produced rename events — Group 6 deferred entirely

**Why this matters:**

- gbrain_id is already a data-fidelity guard even before pages.uuid becomes fully non-optional
- Honest coverage prevents accidental false confidence in incomplete rename logic
- Round-trip tests survive rename implementation without false-positive regressions

**Validation:**

- cargo test --quiet: all 439 tests pass
- cargo clippy --quiet -- -D warnings: clean
- Default model validation: green
- Online-model validation: green

**Next coverage focus:**
- Batch F: direct tests for rename inference outcomes (UUID → page_id preservation, hash ambiguity → quarantine)
- Later: watcher-native event seam once Group 6 lands
- Batch 1 watcher-reliability coverage map (2026-04-27): the `.quaidignore` lane is now split cleanly: parser-level semantics live in `src/core/ignore_patterns.rs`, while watcher delivery into reload/reconcile is covered directly in `src/core/vault_sync.rs`. The remaining Batch 1 proof debt is overflow recovery timing/gating, native→poll fallback, crash backoff/restart, and watcher-health surfacing. I landed low-conflict guard tests in `src/commands/collection.rs` for the restoring-vs-pending-attach-vs-active-reconcile CLI status split and in `src/core/vault_sync.rs` for `memory_collections.restore_in_progress` only flipping true after a real restore ack (`state='restoring'` + `restore_command_id` + `watcher_released_at`).

## 2026-04-24 Vault-Sync 13.5 Slice Review

Reviewed only the 13.5 MCP read-filter slice on `spec/vault-sync-engine`. `brain_search`, `brain_query`, and `brain_list` now accept optional `collection`, explicit names fail clearly when missing, defaulting follows sole-active-else-write-target, and the CLI/write paths remain unchanged apart from passing the new internal `None` filter parameter. Ran `cargo test --quiet mcp::server`; all scoped MCP server tests passed. Verdict: APPROVE.

## 2026-04-24 Vault-Sync 13.5 Slice Re-review

- Verdict: **APPROVE**
- The only real hole Nibbler found is now sealed: `brain_query` threads the effective MCP collection filter into `progressive_retrieve()`, `outbound_neighbours()` enforces that filter in SQL during `depth="auto"` expansion, and the new direct MCP regression test proves a filtered query cannot leak across collections through linked-page expansion. The rest of the 13.5 surface stays narrow and honest: `brain_search`, `brain_query`, and `brain_list` still share the same read-filter/default resolver, while CLI paths continue to pass `None` and no write-path or wider-scope claims were added.
- **Proof seam narrowness guards data-loss surfaces (2026-04-25):** Watcher/quarantine coverage audit landed two epoch-regression proofs and one lifecycle source-invariant proof without widening into restore. The hard-delete guard coverage (every delete path consults DB-only-state) is the highest-value remaining gap. The watcher overflow fallback path (TrySendError::Full branch) is secondary. Restore tests remain deferred until Fry lands no-replace install + post-unlink fsync durability proofs.
- Reconciler/fs-safety lane check at head `7804234` (2026-04-25): current head already contains coordinator commit `03d932e`, so the clustered Unix-lane expectation repairs are present in source with no further Scruffy test edits needed. `src/core/reconciler.rs` now matches the reconciler truth seam (`walked/new == 2` for the real file plus the non-symlink file under the real directory, modified-page compiled truth is asserted with `trim_end()`, the dirty-recheck test still pins fresh-connection TOCTOU ordering, and the 500-file chunk test truthfully expects zero committed pages because invalid `gbrain_id` is rejected during pre-apply tree scanning before any chunk transaction opens); `src/core/fs_safety.rs` also keeps the symlink-root error kind tolerant across Unix kernels (`InvalidInput | NotADirectory`).


- Batch 1 coverage audit complete (2026-04-27T23:51:40Z): Guard tests landed with low conflict. Honest >90% coverage cannot be claimed due to pending implementation-coupled proof for tasks 6.7a, 6.9, 6.10, 6.11. Coverage audit recorded in .squad/orchestration-log/2026-04-27T23-51-40Z-scruffy.md.
- Batch 1 coverage lift (2026-04-28): the honest Windows coverage command is `cargo llvm-cov --lib --tests --summary-only`, not the bare default invocation. After repairing CLI binary discovery for coverage runs (`tests/common/mod.rs`) and stale lib tests in `src/commands/call.rs` / `src/commands/timeline.rs`, the repo measures **84.51%** line coverage (`20,190 / 23,891`), still **1,312 covered lines short** of a 90% gate. The remaining gap is concentrated in `src/commands/collection.rs` (46.07%), `src/core/reconciler.rs` (59.71%), and `src/core/vault_sync.rs` (77.90%), so Batch 1 is not truthfully shippable from this lane without a dedicated backfill sprint across those three files.
- Coverage-safe CLI tests (2026-04-28): subprocess tests that invoke `quaid` should resolve the binary through `tests/common/mod.rs::quaid_bin()` rather than raw `env!("CARGO_BIN_EXE_quaid")`. `cargo llvm-cov` on this Windows lane leaves some integration suites with a missing direct env path even though sibling `target\llvm-cov-target\debug\deps\quaid.exe` exists; the fallback helper keeps `tests/collection_cli_truth.rs`, `tests/quarantine_revision_fixes.rs`, and `tests/search_hardening.rs` runnable under both plain `cargo test` and coverage runs.
