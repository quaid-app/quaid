- **Status:** Two agent lanes completed; one in progress (Fry's own assertions slice); one in final review (Professor)
- **Graph revision (Leela):** All three prior blockers from Professor rejection now resolved. Directionality contract confirmed outbound-only (per spec); temporal gate now includes `valid_from`; CLI test coverage now captures actual text/JSON output shape via `run_to<W>` refactor. **Landing ready.**
- **Assertions/check coverage (Scruffy):** Pure helper seam confirmed; manual assertions preserved across re-index; coverage deterministically validates without stdout capture. **Landing ready.**
- **Fry's own assertions slice:** Currently reconciling compilation errors in assertions/check implementation lane. No blockers reported yet.
- **Professor's graph re-review:** Re-review completed 2026-04-15T23:15:50Z. Verdict: APPROVE FOR LANDING (graph slice tasks 1.1–2.5 only). Issue #28 scope caveat: progressive-retrieval budget/OCC review lane remains separate.
- **Decision merger completed:** Inbox decisions merged into canonical decisions.md (4 files, 0 conflicts). Cross-agent histories updated; orchestration logs written; session log recorded; ready for git commit.

## 2026-04-15 Phase 2 Graph Fix Batch Complete

- **Professor (completed):** Parent-aware tree rendering of `gbrain graph` output (commit `44ad720`). Multi-hop edges render beneath actual parent, not flattened under root. Depth-2 integration test strengthened with exact text shape assertions. All validation gates pass.
- **Scruffy (completed):** Self-loop and cycle render suppression (commit `acd03ac`). Root no longer appears as its own neighbour in human output, even in edge-case cycles. Traversal safety unchanged (visited set). Regression tests cover both states. All validation gates pass.
- **Nibbler (in progress):** Final adversarial re-review of graph slice (tasks 1.1–2.5) after both fixes. Awaiting completion before Phase 2 sign-off.
- **Fry (in progress):** Progressive retrieval slice (tasks 5.1–5.6) and assertions/check slice (tasks 3.1–4.5) both implemented. All 193 tests pass (up from 185). Token-budget logic and contradiction dedup verified. Decisions merged into canonical ledger.

## 2026-04-15 PR #22 Copilot Review Fixes

Applied 13 fixes from Copilot reviewer feedback on PR #22:

1. cargo fmt — resolved all formatting diffs
2. progressive_retrieve error fallback — returns original hybrid_search results instead of empty on error
3. progressive_retrieve budget — caller token_budget overrides config default when non-zero
4. progressive first-result overflow — removed exception that always included first result even if over-budget
5. TemporalFilter::Active doc — corrected to document both valid_from and valid_until checks
6. Graph ORDER BY — added deterministic ordering to outbound SQL
7. Gaps timestamps — human output now shows detected_at and resolved_at
8. MCP depth normalization — case-insensitive auto matching via trim+lowercase
9. MCP temporal synonyms — current/history accepted alongside active/all
10. Contradiction dedup — resolved contradictions no longer block re-detection
11. TempDir leak — replaced mem::forget with :memory: DBs in graph.rs and progressive.rs tests
12. MCP link_id — queried actual id instead of hard-coding 1
13. Test renames — renamed misleading test names in check.rs and tests/graph.rs

All 533 tests pass. cargo fmt, cargo test, cargo clippy all green.

## Learnings

- unwrap_or_else with unused error triggers clippy; use unwrap_or when the fallback is a simple value
- TempDir leak via mem::forget is a pattern to avoid; :memory: is better for unit tests
- Contradiction dedup should only match unresolved rows; resolved contradictions must allow re-detection
- MCP tool parameter matching should always normalize case before string comparison

- Phase 3 OpenSpec (p3-skills-benchmarks) scoped and authored: 5 skill completions, 4 CLI stubs, 4 MCP tools, benchmark harnesses
- p3-polish-benchmarks covers release/docs/coverage only — separate from this feature work
- 4 MCP tools remain unimplemented: brain_gap, brain_gaps, brain_stats, brain_raw (all Phase 3)
- validate.rs uses modular checks (--links, --assertions, --embeddings, --all) for targeted integrity verification
- Benchmark strategy: Rust for offline CI gates, Python for advisory API-dependent benchmarks (LongMemEval, LoCoMo, Ragas)
- Phase 3 wave 1 (groups 2-4) completed: all 4 CLI stubs replaced with working implementations (validate, call, pipe, skills), 4 MCP tools added (brain_gap, brain_gaps, brain_stats, brain_raw), --json wired for validate/skills. 273 tests passing.
- `call.rs` uses a central `dispatch_tool()` function that maps tool names to MCP handler methods — reused by `pipe.rs` for JSONL streaming
- `#[tool(tool_box)]` macro doesn't make methods pub — had to add explicit `pub` to all 16 brain_* methods for call.rs dispatch
- `skills.rs` resolves skills in 3 layers: embedded (./skills/) → user-global (~/.gbrain/skills/) → local working directory, with later layers shadowing earlier ones
- validate tests that create dangling FK references must use `PRAGMA foreign_keys = OFF` to insert then delete, since FK enforcement prevents direct dangling inserts
- `dirs` crate added as dependency for `skills.rs` home directory resolution

## 2026-04-16T06:25:29Z — Phase 3 Core Complete

**Spawn:** fry-phase3-core (claude-opus-4.6, background, 1913s)

**Work:** Completed Phase 3 groups 2-4 (Skills, Benchmarks, MCP Tools). Replaced all four CLI stubs (validate, call, pipe, skills), added 4 MCP tools (brain_gap, brain_gaps, brain_stats, brain_raw), added 16 new tests, marked 14 tasks done in openspec. Zero todo!() stubs remain in src/commands/. Clippy and fmt clean.

**Decisions:** 4 decisions logged to .squad/decisions.md (call.rs dispatch, pub methods, dirs crate, INSERT OR REPLACE).

**Status:** ✅ Ready for Phase 3 review cycle.

## 2026-04-17 Phase 3 CI Integration Final

**What was done:**
- Verified and extended benchmarks job in `.github/workflows/ci.yml` (task 7.1 implementation)
- Fixed two ship-gate regressions before archival:
  1. Added missing `benchmarks` job to ci.yml (task 7.1 noted complete but job was absent)
  2. Fixed 2 clippy violations in tests/concurrency_stress.rs (doc-overindented-list-items, let-and-return)
- All 8 SKILL.md files verified production-ready (no stubs)
- 16 MCP tools registered and tested
- Ship gate 8 validated clean

**Outcome:** Phase 3 implementation complete. All reviewer gates passed. Ready for v1.0.0 tagging.

**Decision file:** `.squad/decisions/inbox/fry-phase3-final.md`

## 2026-04-16T14:59:20Z Simplified-install v0.9.0 Release — Fry Completion

- **Task:** Fixed publish-npm workflow bugs blocking v0.9.0 release
- **Changes:**
  1. `publish-npm.yml` tag pattern corrected (glob match for `v[0-9]*.[0-9]*.[0-9]*`)
  2. `--allow-same-version` enabled to prevent duplicate publish failures
  3. Dry-run validation logic updated for release flow
  4. Install surfaces documentation updated
- **Status:** ✅ COMPLETE. Publish workflow now succeeds for v0.9.0 tag. CI confirmed.
- **Orchestration log:** `.squad/orchestration-log/2026-04-16T14-59-20Z-fry.md`

## Learnings

- v0.9.1 dual-release implementation (2026-04-18): Resumed after crash. Key fix: Cargo.toml default features were `["bundled", "online-model"]` but all docs/CLAUDE.md/release notes said `cargo build --release` produces the airgapped binary. Changed default to `["bundled", "embedded-model"]`. Normalized "slim" → "online" in all contract positions (Cargo comments, inference.rs doc comments, tasks.md scope). Descriptive "slim"/"slimmer" in prose (release notes, docs) is acceptable English, not a contract violation.
- The `build.rs` build-time model download is 3-file (~90MB total): config.json, tokenizer.json, model.safetensors from HuggingFace. `GBRAIN_EMBEDDED_MODEL_DIR` env var allows CI to pre-stage the model files and skip the download.
- `compile_error!` macro in inference.rs prevents both `embedded-model` and `online-model` features being enabled simultaneously — enforces the mutual exclusivity at compile time.
- Release workflow uses `--no-default-features --features ${{ matrix.features }}` so it doesn't depend on Cargo.toml defaults. The default features only affect `cargo build` developer experience.
- stale OpenSpec directories (superseded by a renamed/re-scoped change) should be deleted or archived to prevent naming drift from old contract terms leaking into implementation work.
- v0.9.7 release contract repair (2026-04-25): keep one checked-in manifest file (`.github/release-assets.txt`) as the public release truth, then make workflow verification, installer seam tests, checklist wording, and spec docs read or point at that same manifest. On Windows hosts, validate shell release checks with Git-for-Windows `sh.exe`; leave macOS target proof to GitHub Actions when local cross-target C toolchains are unavailable.

## 2026-04-18: Dual Release v0.9.1 Full Implementation

**Scope:** Implement all three platform surfaces (source-build, shell installer, npm package) for dual-release v0.9.1.

**Work:**
- Phase A: Cargo defaults + naming — `default = ["bundled", "embedded-model"]` (airgapped)
- Phase B: npm surface — postinstall.js + bin/gbrain wrapper + correct asset names
- Phase C: CI + installer — release.yml 8-binary matrix + scripts/install.sh GBRAIN_CHANNEL support
- Version bump: 0.9.1 across all surfaces

**Learning:**
- The mutual exclusion pattern (`compile_error!` when both `embedded-model` and `online-model` are active) is a solid safety gate but relies on developers running `cargo check` with all feature combinations. CI should test this explicitly.
- Release workflows should never depend on `Cargo.toml` defaults for feature flags — always use explicit `--no-default-features --features X` so CI is isolated from developer-ergonomic defaults.
- Post-install scripts that write binaries should write to a separate path (`bin/gbrain.bin`) not overwriting a committed wrapper. This lets npm's bin-linking succeed at pack time.

## Learnings

### 2026-04-22 17:03:19 - Vault-Sync Foundation (v5 Schema)

**What worked:**
- Clean schema evolution: v5 adds collections, file_state, embedding_jobs, and extends pages with collection_id, uuid, and quarantined_at
- Version detection in db::open() cleanly rejects v4 DBs with actionable error message
- FTS triggers now exclude quarantined pages via WHERE clause — efficient and correct
- Collections module structure: validators -> CRUD -> parse_slug pipeline is testable independently
- OpKind enum centralizes resolution logic for Read/WriteCreate/WriteUpdate/WriteAdmin operations

**Challenges:**
- Many existing tests assumed v4 schema — updated version assertions and expected table lists
- Need to defer knowledge_gaps.page_id wiring until later slice (gaps.rs integration)
- Platform-specific fd-safety primitives (rustix/nix) deferred to follow-on slice

**Next:**
- Wire collections into actual commands (init, serve, get, put, etc.)
- Implement reconciler and watcher pipeline
- Add platform-gated fs-safety module for Unix

### 2026-04-22 20:55:00 - Vault-Sync Batch D (walk + classify)

**What worked:**
- Reconciler walk can use `ignore::WalkBuilder` for enumeration while still treating fd-relative `walk_to_parent` + `stat_at_nofollow` as the source of truth for every candidate entry
- Symlink safety gets cleaner when symlinked ancestors are handled the same as direct symlink entries: warn and skip, never trust walker metadata alone
- The delete-vs-quarantine seam is safest as a real SQL predicate plus a pure classification step before any later apply/delete wiring

**Challenges:**
- Windows local validation compiles shared code and runs non-Unix tests, but the symlink walk tests remain Unix-gated by design
- Existing assertion extraction used `asserted_by='agent'`; switching import-derived rows to `import` required updating the replacement semantics too

**Next:**
- Wire the same walk/classify output into rename resolution and the later apply pipeline
- Add the eventual extracted-link insertion path with explicit `source_kind='wiki_link'` when that ingest code lands


### 2026-04-22 17:02:27 - Vault-Sync Batch E (UUID lifecycle + rename resolution)

**What worked:**
- UUID generation (UUIDv7) server-side only when frontmatter lacks gbrain_id — no file rewrite drift
- Reconciler rename classification: native event interface → UUID match → conservative content-hash uniqueness with guards → quarantine + fresh create
- Hash-rename guard: body_size_bytes (post-frontmatter trimmed) instead of whole-file size closes template-note exploit
- All 15+ Page construction sites audited and updated for non-optional uuid: String type
- INFO logging on rename inference refusal guards against silent hash-pair failures

**Key decisions locked:**
1. pages.uuid non-optional across ingest, CLI writes, MCP writes, export/import — authoritative page identity
2. If gbrain_id in frontmatter: adopt only if real UUID and no conflict with stored UUID
3. If no gbrain_id: generate UUIDv7 server-side, store in pages.uuid only (read-only ingest by default)
4. Reconciler rename: strict order (native → UUID → hash), guard on body ≥64 bytes after frontmatter, quarantine on ambiguity

**Tests added:**
- No self-write on default ingest (gbrain_id preserved)
- Rename via native events preserves pages.id
- Rename via UUID match preserves pages.id across directory reorganization
- Rename via content-hash uniqueness preserves pages.id

### 2026-04-22 23:40:00 - Vault-Sync Batch H (restore/remap safety helpers)

**What worked:**
- The restore/remap safety slice is easiest to keep honest as callable core helpers: UUID-migration preflight, RO-mount gate, dirty/sentinel status, drift capture, stat-only stability, fence, and fresh-connection TOCTOU recheck each test cleanly when exposed as separate seams.
- Closed authorization stays reviewable when the enum carries identity strings (`restore_command_id`, lease session id, attach command id) and validation happens before any root open or walk.
- The trivial-content predicate should stay single-sourced with the rename guard (`body_size_bytes < 64 || empty`) so restore/remap preflights and rename inference cannot drift apart.

**Challenges:**
- Reconciler had two subtle double-read seams (`fs::read` plus `hash_file(path)`) that could split `raw_imports` bytes from stored sha256 under concurrent writes; hashing the in-memory bytes closed both.
- Fresh-attach can be implemented honestly at core level now, but higher-level serve/supervisor choreography is still a separate slice and should not be over-claimed in task updates.

**Next:**
- Land Phase 4 remap verification and the real restore/remap execution/orchestration call sites on top of these helpers.
- Wire the attach helper into the future serve/first-use-after-detach entry points without weakening the write gate.
- Ambiguous hash-pair refusal quarantines old, creates new
- Trivial-content (empty body post-frontmatter) never hash-paired

**Validation:**
- cargo test --quiet: all 439 tests pass
- cargo clippy --quiet -- -D warnings: clean
- Default model validation: green
- Online-model validation: green

**Gate results:**
- Professor: APPROVE (UUID wiring truthful, Page.uuid non-optional, defaults safe)
- Nibbler: REJECT initial → APPROVE after Leela repair (body-size guard now safe)
- Leela repair: narrowly fixed hash-rename guard (template-note exploit closed)

**Next:**
- Merge Batch E PR
- Move to Batch F: apply pipeline + raw_imports rotation + full_hash_reconcile

### 2026-04-22 23:30:00 - Vault-Sync Batch F (apply pipeline + raw_imports rotation)

**What worked:**
- Shared `core::raw_imports` helpers made the invariant practical: ingest, directory import, and reconciler apply now rotate bytes inside the same SQLite transaction as page/file_state mutation
- Reconciler can now move from dry-run classification to real apply behavior while preserving `pages.id` across rename matches and re-checking DB-only state at delete time
- Chunking apply work into 500-file transactions is testable with a deliberate second-chunk failure: first chunk commits, later chunk rolls back

**Key decisions locked:**
1. Batch F only wires raw_import rotation for ingest/import/reconcile paths in scope; `brain_put` / UUID self-write hooks stay deferred with their later write surfaces
2. Delete-vs-quarantine must be decided inside the apply transaction, not trusted from an earlier snapshot
3. `embedding_jobs` enqueue is the write-side primitive for reconciler apply; immediate worker/drain behavior remains later work

**Tests added:**
- raw_import rotation keeps exactly one active row, enforces keep/TTL GC, honors KEEP_ALL, and rejects zero-active historical corruption
- ingest/import_dir/reconciler write-path tests assert the active-row invariant after commit
- reconcile apply tests cover hard-delete, quarantine for every DB-only branch, hash-rename apply, and 500-file chunk commit boundaries

**Validation:**
- `cargo test --quiet`: green
- `cargo clippy --all-targets --locked -- -D warnings`: green
- `GBRAIN_FORCE_HASH_SHIM=1 cargo test --quiet --no-default-features --features bundled,online-model`: green
- `cargo clippy --all-targets --no-default-features --features bundled,online-model --locked -- -D warnings`: green
### 2026-04-22 23:55:00 - Vault-Sync Batch I (restore/remap orchestration)

**What worked:**
- `collection_owners` + `serve_sessions` can stay the single live-ownership contract while still mirroring the current lease identity onto `collections.*_lease_session_id` for the existing full-hash authorization checks.
- A lightweight serve runtime loop can satisfy the Batch I handshake/recovery seam without introducing watcher product scope: exact `(session_id, reload_generation)` acks, startup do-not-impersonate, and RCRT-style attach/finalize passes all live in one place.
- Centralizing collection-aware slug resolution and the OR-composed write gate in shared helpers made it practical to harden CLI and MCP mutators together.

**Challenges:**
- Existing command/MCP code assumed global slug uniqueness; introducing collection-aware write routing required touching every mutator that resolves page IDs directly.
- The CLI and MCP `brain_put` contracts differ: CLI still allows unconditional upsert, while MCP must fail closed unless `expected_version` is supplied for updates.

**Next:**
- Finish the remaining Batch I adversarial proofs (`17.5ii4`, `17.5pp`, `17.5qq2-8`, restore integrity escalation paths) before calling the batch fully closed.
- Vault-sync-engine Batch K1 (2026-04-23): `collection add` should validate the root and parse `.gbrainignore` before inserting any row; then attach via the detached fresh-attach path with a short-lived lease so `collection_owners`, `serve_sessions`, and probe files clean themselves up even on failure. The K1 read-only gate is best kept narrow: persist `collections.writable`, surface `read-only` truthfully in list/info, and use `CollectionReadOnlyError` only on vault-byte-facing write surfaces (`put` in current scope), while slug-bound `brain_gap` still takes the restoring interlock but is not falsely blocked on `writable=0`.

## 2026-04-23T09:02:00Z Batch K2 Final Approval

**Status:** K2 APPROVED FOR LANDING

Offline restore integrity closure is now fully approved by both Professor and Nibbler. Fresh-attach + lease discipline from K1 maintained. Restore originator identity persisted and compared. Tx-B residue durable and auditable. Manifest retry/escalation/tamper behavior coherent. Reset/finalize surfaces truthful. CLI completion path via \sync --finalize-pending -> attach\ proven end-to-end.

Ready for implementation and landing.

## Learnings

### 2026-04-23 22:15:00 - Vault-Sync Batch L1 (startup restore-orphan recovery)

**What worked:**
- Startup recovery stayed honest once `gbrain serve` did the real order synchronously: stale-session sweep, new session registration, lease claim, RCRT pass, then supervisor-handle bookkeeping.
- Treating `supervisor_handles` as explicit startup bookkeeping made the L1 slice narrow enough to land without implying the deferred sentinel-directory recovery work.
- Using the same 15s stale-heartbeat rule for startup recovery kept restore-orphan takeover fail-closed: fresh command heartbeats defer, stale ones recover.

**Challenges:**
- The pre-existing runtime loop was doing startup work lazily in the background, which made the approved startup order only incidental until the recovery path was pulled into `start_serve_runtime()`.
- Windows test timing around spawned binaries can surface transient file-lock noise, so the exact required `cargo test --quiet` lane may need a rerun even when the code is correct.

## 2026-04-24 M1b-ii/M1b-i Session Completion

- **M1b-ii implementation COMPLETE:** Unix precondition/CAS hardening for `gbrain put` / `brain_put`. Real `check_fs_precondition()` helper with self-heal capability; separate no-side-effect pre-sentinel inspection variant for write path to preserve sentinel-failure truth ordering.
- **Scope:** 12.2 + 12.4aa–12.4d only. CAS/precondition failures now rejected before sentinel creation with no DB mutation.
- **Decision:** Kept two-layer precondition split per inbox decision (real helper for deferred full work; write-path uses separate no-side-effect variant). Future full `12.1` closure must preserve ordering: pre-sentinel inspection first, sentinel before any DB mutation.
- **Validation:** ✅ `cargo test --quiet` passed (Windows default lane). Unix CAS/precondition proofs require cross-check on Linux CI.
- **Inbox decision merged:** Fry M1b-ii precondition split decision now in canonical `decisions.md`.
- **Orchestration log written:** `2026-04-24T12-54-00Z-fry-m1b-ii-implementation-lane.md`.
- **Session log written:** `2026-04-24T12-55-00Z-m1b-session.md`.
- **Status:** Awaiting final Professor + Nibbler gate approval for M1b-ii.

## Learnings

### 2026-04-24 16:35:00 - Vault-sync 13.3 CLI slug parity

**What worked:**
- Keeping collection-aware resolution at the command boundary was the cleanest parity move: resolve once, keep DB lookups keyed by `(collection_id, slug)`, and only canonicalize `<collection>::<slug>` on CLI output.
- Reusing the MCP canonical-output contract for CLI read surfaces (`get`, `graph`, `links`/`backlinks`, `timeline`, `check`, `search`, `query`, `list`) closed the parity seam without widening into the deferred collection-filter/tool work.
- Integration tests that spawn `gbrain` directly were the fastest way to prove the seam end-to-end, especially for ambiguous bare slugs and explicit collection routing.

**Challenges:**
- `check --all` and graph/backlink fixtures had hidden single-collection assumptions; once CLI outputs became canonical, older tests had to be updated to stop asserting bare slugs.
- Slug-bound `check` only recomputes the selected page, so contradiction-output tests need either pre-seeded assertion state or an all-pages warmup pass before checking the explicit-route output.

### 2026-04-24 06:05:00 - Vault-Sync Batch 13.6 / 17.5ddd (`brain_collections` MCP shape slice)

- Added the read-only `brain_collections` MCP surface as a projection helper in `vault_sync.rs`, not as ad hoc JSON assembly in the server, so the frozen 13-field contract lives next to the collection/runtime truth it depends on.
- Kept the tool honest by masking `root_path` to `null` whenever the collection is not `active`, parsing `ignore_parse_errors` into the tagged union the design froze, and surfacing `integrity_blocked` as the new string-or-null discriminator instead of reusing the older CLI-only blocked-state summary.
- `recovery_in_progress` needed real runtime truth instead of guesswork, so I added a narrow process-local recovery registry around `complete_attach(...)`; queued recovery remains `needs_full_sync=true, recovery_in_progress=false`, while active attach hashing flips the runtime bit until the handoff completes. Validation on this Windows host: `cargo fmt --all`, targeted `brain_collections` tests, and full `cargo test --quiet` all passed.

### 2026-04-25 07:25:00 - PR #77 feedback and v0.9.6 ship

- When a platform gate sits at the public command boundary (`gbrain serve`), the docs need to describe the command as gated even if some lower-level helpers compile on other platforms. Reviewer feedback will keep coming back if the docs talk about an internal seam instead of the user-visible one.
- Restore notes have to distinguish between “no implementation” and “narrow landed seam.” For quarantine restore, the honest wording was “Unix-only, no-replace target, pre-existing parent dirs, online handshake still deferred,” not “not yet implemented.”

### 2026-04-25 10:45:00 - Issue #81 watcher empty-root repair

- `src/core/vault_sync.rs` is the right regression seam for serve-only watcher bootstrapping: normalize invalid collection rows there and keep the proof close to watcher selection.
- Cross-platform proof should target the deterministic normalization helper (`detach_active_collections_with_empty_root_path`), while the Unix-only watcher test can stay narrow and just prove the active watcher set excludes blank-root rows.
- The default collection bootstrap is `root_path=''` plus `state='detached'` in `src/core/db.rs`; tests that want the old broken state must opt into it explicitly with `UPDATE collections SET state='active', root_path='' WHERE id = 1`.

### 2026-04-25 11:35:00 - Issue #81 release-ready patch lane

- When a hotfix PR is the next shippable patch, bump every public version truth that users can copy from (`Cargo.toml`, npm package metadata, runtime user agent, README/docs install snippets) in the same change or the release surface drifts immediately.
- GitHub release body text is part of the product contract: keep install commands stable, but rewrite the explanatory paragraph so it names the actual hotfix instead of inheriting stale notes from the prior patch lane.

---

## 2025-07-18 — export test Linux fix (PR #111)

### Problem

`run_exports_page_to_nested_markdown_file` panics on Linux at `put_from_string().unwrap()`
with `No such file or directory (os error 2)`. Root cause: the test's `open_test_db()`
helper created a real on-disk database but left the default collection seeded by
`db::open()` with `root_path = ''` and `state = 'detached'`.

On Unix, `persist_with_vault_write()` (the `#[cfg(unix)]` variant) skips the
early-exit branch (`db_path.is_empty() || db_path == ":memory:"`) because the DB is a
real file, then calls `fs_safety::open_root_fd(Path::new(""))` which returns `ENOENT`.
The `#[cfg(not(unix))]` variant goes straight to `persist_page_record()` and never
touches the filesystem, so Windows CI was green.

### Fix

Provision a real vault directory and update the `collections` row in `open_test_db()`,
mirroring the existing `open_test_db_with_vault()` pattern in `put.rs`. The `TempDir`
is already returned so no `mem::forget` is needed.

### Learnings

1. **Default collection is a stub.** `db::open()` seeds `collections` with
   `root_path=''`, `state='detached'`. Any test that exercises the Unix write path must
   provision a real directory and update the row, or use an in-memory database.

2. **The `:memory:` guard only applies to in-memory DBs.** On-disk test databases always
   go through the full vault write path on Unix, even if no vault was configured.

3. **`put.rs` has the pattern.** `open_test_db_with_vault()` is the reference
   implementation for any test that calls put operations on an on-disk database.

4. **cfg(unix) divergence is a cross-platform failure risk.** Run `cargo test` on Linux
   before shipping put/export test helpers.

## 2026-04-28 — PR #111 baseline port (commit 305d186)

**Learning:** When a feature branch is cut before baseline debt fixes land on main,
the CI gate will fail on the baseline errors — not the feature change itself.
Always check if a failing CI branch diverged before a known mechanical fix wave,
and port only those exact approved fixes without pulling in unrelated logic.

**Learning:** cargo clippy -- -D warnings can be used locally in a worktree to
validate a clean compile before pushing, providing high confidence CI will pass
on the same clippy gate.

**Learning:** .map_err(|e| e.to_string())? silently compiles on Windows if the
outer Result error type happens to be compatible, but fails on Linux when the
clippy -D warnings gate is active and From<String> is not implemented for the
error enum. Always use the explicit variant (ErrorType::Variant { message: e.to_string() })
for map_err where the error type is a custom enum.

**Learning:** PathBuf::from(...) in match guards triggers clippy::cmp_owned.
Use Path::new(...) instead — it borrows without allocating, satisfies
`PartialEq<Path>` on `PathBuf`, and avoids the lint without changing semantics.

## Learnings

- 2026-04-28: For vault-sync watcher bookkeeping, close task truth against the shared reconcile call graph, not against hypothetical per-event handlers. `src/core/vault_sync.rs::poll_collection_watcher()` → `run_watcher_reconcile()` → `src/core/reconciler.rs::reconcile_with_native_events()` is the load-bearing path for create/modify, delete/quarantine, and native rename application.
- 2026-04-28: Batch 1 watcher-health truth lives in `src/core/vault_sync.rs` and `src/commands/collection.rs`; `quaid collection info` is the only surfaced watcher-health contract for v0.10, while MCP `memory_collections` remains intentionally frozen.
- 2026-04-28: User preference reinforced — OpenSpec bookkeeping must be conservative and evidence-based: mark only tasks and ship gates proven by the merged tree, and leave explicit notes wherever scope is still deferred. Key working files for this lane: `openspec/changes/vault-sync-engine/tasks.md`, `openspec/changes/vault-sync-engine/implementation_plan.md`, `.squad/decisions.md`.
Use Path::new(...) instead — it borrows without allocating and satisfies
PartialEq<Path> on PathBuf.

---

## 2025-07-17 — PR #111: Unix clippy dead_code on WatcherHandle

**Task:** Fix Linux CI Check failure on ix/export-nested-markdown-linux (online-model clippy pass).

**Root cause:** #[cfg(unix)] enum WatcherHandle in src/core/vault_sync.rs had two variants (Native(RecommendedWatcher) and Poll(PollWatcher)) held only for Drop semantics. Clippy -D warnings on the online-model channel flagged them as dead_code.

**Fix:** Added #[allow(dead_code)] to both variants and a comment explaining the intent (Drop semantics, not direct reads). Ported exactly from the guardrails branch.

**Commit:** 5152ef7 on branch ix/export-nested-markdown-linux.

**Learning:** When an enum variant holds a type purely for Drop (to keep a resource alive), and that field is never read, always annotate with #[allow(dead_code)]. This is especially common with watcher/handle types from external crates. The online-model feature channel can surface dead_code warnings that the default channel hides, so both clippy passes should be run before merging.
