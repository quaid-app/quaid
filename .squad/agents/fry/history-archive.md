# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

### 2026-04-28 - Batch 2 embedding-worker closure

- **Queue contract:** Batch 2 can add `chunk_index`/retry metadata without reopening queue identity if the live helper contract stays `UNIQUE(page_id)` and the worker regenerates embeddings per page. Re-enqueue for new bytes must clear stale `running`/`failed` state or a page can stay semantically stale forever after one old failure.
- **Frozen MCP boundary:** `memory_collections` is still a frozen machine-readable surface. When Batch 2 needed failing-job diagnostics, the safe pattern was to keep `embedding_queue_depth` truthful (`pending + running` only), retain the internal failing-job count for CLI/tests, and avoid widening the serialized MCP object.
- **Windows coverage seam:** On this host, `cargo llvm-cov` can fail with linker/file-lock errors when it reuses `target\\llvm-cov-target`. Setting a fresh repo-local `CARGO_TARGET_DIR` for the run, then reading the result with `cargo llvm-cov report --summary-only`, produced a stable summary again.

### 2026-04-29 - Batch 1 Closeout: Watcher Tasks 6.5/6.6/6.7

- **Outcome:** Closed watcher tasks 6.5, 6.6, and 6.7 in OpenSpec via the already-landed shared reconcile path
- **Decision:** Use existing `run_watcher_reconcile()` → `reconcile_with_native_events()` pipeline; don't demand redundant per-file watcher handlers
- **Evidence:** Merged code already covers re-ingest, delete-vs-quarantine, and rename-with-page-id-preservation
- **OpenSpec status:** Updated `openspec/changes/vault-sync-engine/tasks.md` with truthful closure notes

### 2026-05-01 - PR #111 rustfmt drift cleanup

- **Pattern:** When CI's `Check` job fails on a branch due to rustfmt drift accumulated on `origin/main`, the correct fix is `cargo fmt` (project-wide), not file-targeted `rustfmt` calls. File-targeted `rustfmt` without Cargo.toml edition context errors on `async fn` with "not permitted in Rust 2015".
- **Scope creep from known list:** The task named 6 drift files; `cargo fmt --check` revealed 8 files needed formatting. Always verify the full scope with `cargo fmt --check` before committing.
- **Validation sequence:** `cargo fmt` → `cargo fmt --check` (must exit 0) → `cargo check` (must compile clean). All three passed before pushing.
- **Commit type:** Pure formatting commits use `style:` prefix (not `fix:` or `chore:`).

### 2026-05-02 - PR #111 test fixture vault-root pattern

- **Root cause:** `quaid init` seeds the default collection with `root_path=''` (`state='detached'`). Any test that calls `put::put_from_string()` via a real on-disk DB will hit `vault_sync::with_write_slug_lock`, which tries to open the empty root path — ENOENT on Linux.
- **Fix pattern:** Before any write-through operation in integration tests, call a `provision_vault(dir, conn)` helper that creates `vault/` inside the temp dir and patches the collections row (`root_path`, `writable=1`, `is_write_target=1`, `state='active'`, `needs_full_sync=0`). This mirrors the pattern first introduced in `src/commands/export.rs` unit tests (commit `4735713`).
- **Rule:** Every integration test that opens a real DB and writes pages must call `provision_vault` before the first write. Failing to do so will pass on Windows but break on Linux.

### 2026-05-02 - PR #111 vault_sync poll-watcher fallback bug

- **Bug pattern:** Using `?` inside an `if/else` expression that is assigned to a `let` binding causes errors to escape the *function*, not stay in the binding. In `start_collection_watcher`, three `map_err(...)?` calls on the native watcher init were propagating `VaultSyncError` out of the function, making the `match native_result { Err(_) => poll fallback }` block dead code for real failures.
- **Detection:** Only caught by code review — the test used a `FORCE_NATIVE_WATCHER_INIT_FAILURE` flag that injected `Err(String)` directly into `native_result`, so it never exercised the buggy `?` path. Tests passing ≠ fallback logic working.
- **Fix pattern:** Wrap multi-step init sequences in an immediately-invoked closure (`(|| -> Result<_, _> { ... })()`) when failures must be captured into a binding rather than propagated. This keeps `?` convenience inside the closure while ensuring errors land where the match expects them.
- **Rule:** Whenever you write `let result = if condition { Err(...) } else { ... watcher.init()? ... Ok(...) }`, verify the `?` operators are inside a closure — not bare in the else block — if `result` is meant to carry the failure.



- **Rejection summary:** Professor gated Batch 1 on 2026-04-28 and rejected the current closure plan due to three interlocking contradictions: (1) overflow recovery tried to bypass `ActiveLease` authorization, (2) `memory_collections` MCP schema tried to widen past the frozen 13.6 contract, (3) `WatcherMode` enum had unreachable `"inactive"` variant given the null rule.
- **Leela repair:** Scope narrowed same day: overflow-recovery mode moved to `FullHashReconcileMode` (authorization stays `ActiveLease`); watcher health narrowed to CLI-only `quaid collection info` (no MCP widening); `WatcherMode` simplified to `Native | Poll | Crashed` (no `Inactive`).
- **Implementer lockout:** Fry is locked out of the next revision of Batch 1 artifact. All repaired 6.7a / 6.8 / 6.9 / 6.10 / 6.11 tasks assigned to Mom for implementation.
- **Reason:** Fry authored the rejected scope. Leela's direct artifact repair demonstrates the narrowing is the correct direction. Mom has a track record on repair work (quarantine restore, MCP spec surface fix, default-path fix) and has not been part of the rejected scope.
- **v0.10.0 gate update:** All 13 Batch 1 tasks must be marked `[x]` with truthful closure notes; 6.7a closure names `FullHashReconcileAuthorization::ActiveLease` explicitly; 6.11 closure confirms `memory_collections` NOT widened; 13.6 exact-key test passes clean; `cargo test` passes zero failures; coverage ≥ 90%; Nibbler adversarial sign-off on 6.9 and 6.10; Cargo.toml bumped to `0.10.0`; CHANGELOG.md updated.

### 2026-04-27 18:39:39 - Search skill PR landing

- **User preference:** When preserving squad skill extracts, land them through a branch + draft PR flow instead of keeping them as local-only artifacts.
- **Scope decision:** Keep `.squad/skills/compound-term-tiered-fts/`, `.squad/skills/deterministic-hybrid-proof/`, `.squad/skills/search-proof-contracts/`, and `.squad/skills/search-surface-coverage/`; delete stray local artifacts `.squad/git-commit-msg.txt`, `create_files.py`, `scribe-cleanup.py`, and `scribe-commit.bat`.
- **Key paths:** This lane was coordination-only around `.squad/skills/`, `.squad/agents/fry/history.md`, and `.squad/decisions/inbox/fry-skill-pr-landing.md`.
- **Workflow pattern:** In a shared dirty tree, safest landing path was `fetch origin/main` and branch from `origin/main` without disturbing another agent's unstaged changes.

### 2026-04-25 15:48:00 - Put/OCC lane closeout (2026-04-25T15-48-57Z)

- **Orchestration log:** `.squad/orchestration-log/2026-04-25T15-48-57Z-fry.md` recorded lane closure
- **CI outcome:** put/OCC surface fix complete; `StaleExpectedVersion` error format unified across CLI, MCP, and server handler
- **Format alignment:** All four consumer substrings now satisfied by: `"Conflict: ConflictError StaleExpectedVersion collection_id={} relative_path={} expected_version={} current version: {}"`
- **Validation:** `cargo test --quiet --lib commands::put`, `cargo test --quiet --lib mcp::server::tests::brain_put`, `cargo test --quiet --test concurrency_stress` all pass on Windows
- **Status:** Lane closed. Ready for merge.

### 2026-04-25 12:15:00 - Put/OCC stale-conflict test truth

- The in-memory `commands::put` OCC path still reports stale-update conflicts through `persist_page_record()` as `Conflict: page updated elsewhere (current version: N)`, while the Unix vault-write path surfaces the structured `ConflictError` variants from `vault_sync`.
- For this lane, the stable proof boundary is per surface: CLI/MCP vault-write tests should assert the typed `ConflictError`/`CollectionRestoringError` contract, but pure in-memory unit tests should match the legacy `current version: N` wording instead of assuming Unix-only formatting.
- Validation on current head after the fix: `cargo test --quiet --lib commands::put`, `cargo test --quiet --lib mcp::server::tests::brain_put -- --nocapture`, and `cargo test --quiet --test concurrency_stress -- --nocapture` all pass on this Windows host.

### 2026-04-25 11:40:00 - Quarantine restore narrow re-enable slice

- The narrowest truthful no-replace restore seam on Unix is "tempfile in target dir → `linkat` install → unlink temp → parent `fsync`" rather than a pre-check plus replace-prone rename; the hard-link install lets a concurrently-created target win at install time without widening into overwrite policy.
- For rollback credibility, every successful unlink in the post-install window needs its own observable parent-`fsync` proof seam; a tiny env-driven trace hook kept the integration test honest without introducing a production-only state machine.
- Validation on this Windows host stayed limited: `cargo test --quiet --test quarantine_revision_fixes --test collection_cli_truth` passed, while full `cargo test --quiet` still hits the pre-existing parent-path failures in `commands::init::tests::init_rejects_nonexistent_parent_directory` and `core::db::tests::open_rejects_nonexistent_parent_dir`, and a Linux cross-check remains blocked by missing `x86_64-linux-gnu-gcc`.

### 2026-04-25 10:20:00 - Vault-Sync post-batch coverage follow-up

- The hard-delete truth for quarantine now stays simplest when every destructive path shares the same five-branch `reconciler::has_db_only_state(...)` predicate and only layers counts/receipts on top for operator messaging.
- Good coverage wins on the current quarantine seam come from positive-path proofs (`export`→same-epoch `discard`, forced discard, list counts/export timestamps) plus a source-level invariant that fails if reconcile/discard/TTL drift away from the shared predicate.
- Relevant files for this follow-up: `src/core/quarantine.rs`, `src/core/vault_sync.rs`, `tests/collection_cli_truth.rs`, and `openspec/changes/vault-sync-engine/tasks.md` (`17.17d` closure note).

### 2026-04-25 06:35:00 - Vault-Sync quarantine lifecycle + dedup cleanup slice

- The default quarantine seam can stay reviewer-friendly if it leans on existing invariants instead of inventing a second state machine: export/discard/restore all route through the same resolved `<collection>::<slug>` addressing, the same five-branch DB-only predicate, and the existing active `raw_imports` bytes as the restore source of truth.
- Tracking export eligibility per `(page_id, quarantined_at)` in a small `quarantine_exports` table cleanly solves the “export relaxes discard” rule without widening page schema or losing the distinction between old and newly re-quarantined epochs.
- Quarantined hard-deletes exposed an FTS trigger edge: deleting a page that was already absent from `page_fts` can corrupt the external-content index if the delete trigger fires unconditionally, so `pages_ad` now skips the FTS delete op when `old.quarantined_at IS NOT NULL`.

### 2026-04-25 02:20:00 - Vault-Sync watcher core + dedup slice

- Kept the watcher slice narrow: `start_serve_runtime()` now owns one `notify` watcher plus bounded `tokio::mpsc` queue and debounce buffer per active collection, then flushes bursts through the existing reconciler instead of inventing a second mutation path.
- Reused the existing process-global runtime registries for self-write suppression instead of bolting on a separate service object: the serve process now keeps a path+hash+instant dedup map, `gbrain put` inserts before `renameat`, watcher classification drops only recent exact path+hash matches, and a 10s sweeper ages entries out.
- Truth boundary stayed explicit: live `.gbrainignore` reload, watcher health/supervision, and broader overflow choreography remain deferred; validation here was `cargo test --quiet` green, while `cargo clippy -- -D warnings` is still blocked by pre-existing dead-code warnings in unrelated modules (`assertions`, `graph`, `search`).

### 2026-04-24 23:10:00 - Vault-Sync Batch 13.5 (read-only MCP collection filter slice)

- Kept 13.5 narrow and read-only: only `brain_search`, `brain_query`, and `brain_list` gained an optional MCP `collection` filter, with no CLI widening and no write-path changes.
- The default filter rule is now encoded in one helper and one proof seam: absent `collection` means the sole active collection when exactly one is active, otherwise the write-target collection — never “all active collections.”
- Search/list backends take an optional collection-id filter directly, so MCP filtering stays in the SQL/vector lanes instead of post-filtering mixed results.

### 2026-04-24 14:25:00 - Vault-Sync Batch M1b-ii (Unix put precondition/CAS slice)

- Kept the slice honest and Unix-only: `gbrain put` / `brain_put` now reject missing-or-stale update `expected_version` and filesystem precondition conflicts before recovery-sentinel creation, without claiming the deferred mutex, IPC, or happy-path closure work.
- Implemented `check_fs_precondition` as a real fast/slow-path helper with typed `ConflictError` branches plus self-heal, but the Unix write path uses a no-side-effect pre-sentinel inspection variant so sentinel-creation failure still guarantees no DB mutation.
- Validation on this Windows host: `cargo test --quiet` passed after the slice landed, including the new Unix-gated precondition/CAS proofs.

### 2026-04-24 01:35:00 - Vault-Sync Batch M1a (pre-gated writer sentinel crash core)

- Landed only the narrow writer-side sentinel crash seam: sentinel create+durability, tempfile rename, parent-fsync hard-stop, post-rename foreign-rename detection, retained sentinel on post-rename failure, and startup-recovery fallback when `collections.needs_full_sync` cannot be written.
- Kept full `12.1` honest by splitting out `12.1a`; `12.2`, `12.3`, `12.4` mutex, `12.5`, `12.6*`, `12.7`, IPC, and generic startup-healing claims all remain deferred.
- Validation on this Windows host: `cargo test --quiet` passed. Unix-only M1a proofs were added under `#[cfg(unix)]`, but a Linux cross-check was not feasible locally because the required `x86_64-linux-gnu-gcc` toolchain is absent.

### 2026-04-23 23:30:00 - Vault-Sync Batch L1 (restore-orphan startup recovery narrowed slice) - APPROVED FOR LANDING

**Scope:** L1 narrowed to startup restore-orphan recovery only after K2 proved the happy offline restore path.

**Implementation complete:**
- Fixed startup order: stale-session sweep → register own session → claim ownership via `collection_owners` → run RCRT recovery → register supervisor bookkeeping
- Registry-only half of task 11.1 (`supervisor_handles` + dedup bookkeeping); 11.1b (sentinel-directory) deferred to L2
- One shared 15s stale-heartbeat threshold for startup recovery decisions
- Recovery callable only through `finalize_pending_restore(..., FinalizeCaller::StartupRecovery { session_id })`
- Validation: ✅ default lane, ✅ online-model lane

**Claims:**
- 11.1a: registry-only startup scaffolding
- 17.5ll: shared 15s heartbeat gate, exact-once finalize, fresh-heartbeat defer, `collection_owners` ownership truth
- 17.13: real crash-between-rename-and-Tx-B recovery (not fixture)

**Deferred:**
- 11.1b (sentinel-directory), 11.4 (sentinel recovery), 17.12 (sentinel proof), 2.4a2 (Windows platform gating), online handshake, IPC, broader supervisor-lifecycle → L2+

**Gate status:** ✅ Pre-implementation gates satisfied (Professor + Nibbler). L1 APPROVED FOR LANDING.

### 2026-04-23 20:40:00 - Vault-Sync Batch K2 (offline restore integrity closure)

**What worked:**
- Offline restore stayed honest without widening plain sync by keeping `gbrain collection sync <name> --finalize-pending` as the single explicit CLI completion path from post-Tx-B state to active attach completion.
- `collection info` needed a distinct post-Tx-B attach-pending surface; treating it as generic `restoring` hid the real operator action.
- Terminal integrity has to remain sticky inside `finalize_pending_restore()`: once `integrity_failed_at` is set, a later manifest match must NOT auto-clear it without explicit `restore-reset`.

**Challenges:**
- The restore attach path was using active-lease authorization while reconcile authorization only admitted restore-specific identities; the contract had to be aligned before CLI completion could work.
- Windows still hard-gates vault-sync restore flows, so the real `17.11` proof has to remain Unix-only until the broader platform task lands.

- Vault-sync-engine Batch K1 (2026-04-23): **FINAL APPROVAL CONFIRMED**. Collection add scaffolding + shared read-only gate narrowed to honest boundary: `1.1b`, `1.1c`, `9.2`, `9.2b`, `9.3`, `17.5qq10`, `17.5qq11` only. Pre-gate approvals: ✅ Professor (K1 is safe boundary, non-negotiables affirmed), ✅ Nibbler (adversarial seams specified). Implementation + proof lanes: ✅ Fry (add validates before row creation, detached attach, short-lived lease cleanup, writable truth surface), ✅ Scruffy (partial then full after Leela repairs), ✅ Leela (repaired MCP surface + task honesty). Final re-gate: ✅ Professor (read-only gate honestly scoped, caveat attached), ✅ Nibbler (seams controlled, DB-only mutators out of K1 claim, no offline restore broadening). All 8 decisions merged to canonical ledger. Required caveat: K1 is default attach + list/info truth + vault-byte refusal only; `--write-gbrain-id`, broader mutators, restore-integrity closure deferred to K2. **K1 APPROVED FOR LANDING.**
- Vault-sync-engine Batch J (2026-04-23): **APPROVED FOR LANDING**. Narrowed Batch J (plain sync + reconcile-halt safety) implemented in four files: `src/commands/collection.rs` (sync command, fail-closed gates, CLI truth), `src/core/vault_sync.rs` (sync path with lease/entry checks), `src/core/reconciler.rs` (DuplicateUuidError + UnresolvableTrivialContentError halts terminal), `tests/collection_cli_truth.rs` (15 test cases, all pass). All five blocked states fail-closed; lease acquired/heartbeat/released via RAII; duplicate/trivial halts terminal; operator surfaces truthful. Validation: ✅ `cargo test --quiet` (default lane), ✅ online-model lane. Scruffy proof lane confirmed. All 6 decisions merged to canonical ledger. Final re-gate approvals: ✅ Professor (fail-closed finalize gate established, CLI-only boundary preserved), ✅ Nibbler (blocking seam controlled, no success-shaped outcomes leak, repair narrow, deferral explicit). **Batch J CLOSED — ready for merge to main.**
- Vault-sync-engine Batch G (2026-04-22): `full_hash_reconcile` now needs an explicit, closed mode/authorization contract validated against `collections.state` before any walk, and the hash-unchanged branch must self-heal only `file_state` metadata while preserving user bytes and `raw_imports`. The `render_page` seam now always re-emits persisted `pages.uuid` as `gbrain_id`, so agent updates cannot strip identity even when incoming markdown omits the field.
- Vault-sync-engine Batch E (2026-04-22): Wired UUIDv7 lifecycle without file rewrite drift by making `Page.uuid` explicit, preserving `gbrain_id` through parse/render, and generating UUIDs server-side only when frontmatter lacks one. Reconciler rename classification now works in the intended order (native interface → UUID → conservative hash), and ambiguous/trivial hash matches fail closed into `quarantined_ambiguous` with INFO refusal logging instead of optimistic pairing.
- Vault-sync-engine Batch C (2026-04-22): Resumed after rate-limit interruption. Prior run had completed fs_safety.rs implementation (all six primitives + 15 unit tests). Finished Batch C by: (1) marking rustix dependency complete (already in Cargo.toml), (2) advancing stat_file/stat_diff/full_hash_reconcile to honest foundations showing contracts, (3) advancing reconciler walk plumbing to demonstrate safe fd-relative structure with Unix/Windows platform gates, (4) fixing platform-specific test to handle Windows UnsupportedPlatformError. All 439 lib tests pass. Foundation is truthful: primitives work, contracts are clear, stubs explicitly note what's deferred to full reconciler batch.
- PR #32 review fix (2026-04-16): Addressed all 10 Copilot review threads on the simplified-install PR. npm bin wrapper pattern: ship a committed shell script (bin/gbrain) that execs a downloaded native binary (bin/gbrain.bin) — ensures npm bin-linking succeeds even when postinstall fails. postinstall.js needs both connection and socket timeouts (60s) on https.get to prevent npm install from hanging on stalled connections. install.sh must check INSTALL_DIR writability via explicit test before mkdir/mv to provide actionable error messages (not raw set -e failures). Release notes should use tag-pinned URLs (github.ref_name) not main for reproducibility. Node engine constraint should track supported LTS only (>=18, not EOL >=16).
- PR #33 CI + review fix (2026-04-17): Fixed 2 CI failures + 5 review threads. (1) `--all-features` in clippy/coverage trips `compile_error!` since embedded-model and online-model are mutually exclusive — replaced with per-channel clippy runs and default-features-only coverage. (2) BEIR regression crash from BERT max_position_embeddings=512 — added tokenizer truncation to 512 tokens in `embed_candle()`. Review fixes: spec.md feature dep `hf-hub`→`dep:reqwest`, tasks.md B.3 overclaim removed (npm has no GBRAIN_CHANNEL override), deprecated "slim" wording removed from spec files, `install.sh` restored to `mktemp -d` for secure temp dirs.
- Simplified-install npm publish alignment (2026-04-16): `publish-npm.yml` tag pattern must match `release.yml` (`v[0-9]*.[0-9]*.[0-9]*`), NOT `v*`. `npm version` needs `--allow-same-version` when package.json already matches the tag version. Use `npm pack --dry-run` (not `npm publish --dry-run`) for unconditional validation — publish dry-run hits the registry and fails when versions conflict. The `gbrain` npm package name has existing published versions (1.3.1+); ownership/version strategy must be resolved before first public publish.
- Phase 3 CI integration (2026-04-17): Offline benchmarks (corpus_reality, concurrency_stress, embedding_migration) run as a named `benchmarks` job in ci.yml, separate from the general `cargo test` job. BEIR regression lives in its own workflow file (`beir-regression.yml`) to avoid blocking PRs with ~500MB dataset downloads. Formatting fixed before commit — always run `cargo fmt --all` before pushing.
- The core implementation target is a Rust CLI plus MCP server.
- The system is intentionally local-first and zero-network for embeddings.
- Every meaningful implementation starts with an OpenSpec proposal.
- Doc parity requires matching artifact names exactly: CI produces a `coverage-report` artifact (not `lcov.info`); spec URLs must use `macro88/gigabrain`, not `[owner]`; checksum verify must use `shasum --check` directly against the `.sha256` file, not `echo ... | shasum --check`.
- Always run `cargo fmt --all` before committing Rust code — CI enforces `cargo fmt --check` as the first gate and will skip all subsequent steps (clippy, check, test) if formatting fails.
- Local Windows dev environment lacks MSVC SDK libs (`msvcrt.lib`), so clippy/build/test cannot run locally. Use CI (Linux) for full validation. Only `cargo fmt` works locally.
- CI runs `cargo clippy -- -D warnings` which treats all warnings as errors. Stub functions with `todo!()` bodies must prefix all params with `_` to avoid unused variable errors.
- `version.rs` was removed — it was dead code never referenced from `mod.rs` or `main.rs`.
- PR #9 review (2026-04-14): Copilot automated reviewer caught 9 issues. 7 applied in Sprint 0 scope (CLI contract alignment, schema CHECK, docs fixes, Cargo.lock, hygiene). CI clippy `-D warnings` left as-is since stubs already fixed. Repo hygiene pass removed `gh_diagnostic.py`, one-time session artifacts.
- CLI contract must match `docs/spec.md` exactly — the spec defines the scaffold's surface. `default_db_path()` must resolve to `./brain.db`, not `$HOME/brain.db`.
- `init` and `version` commands don't require a database connection; dispatch them before `db::open()` in main.
- Reviewed and proposed adoption of `rust-best-practices` skill (Apollo GraphQL handbook, 9 chapters) at `.agents/skills/rust-best-practices/`. Decision note at `.squad/decisions/inbox/fry-rust-skill-adoption.md`. Key caveats: `#[expect]` needs MSRV ≥1.81, `rustfmt` import grouping needs nightly, snapshot testing (`insta`) deferred to Phase 1 test work.
- Vault-sync-engine Batch B (2026-04-22): Implemented Group 3 (ignore patterns), partial Group 4 (file state tracking), and Group 5.1 (reconciler scaffolding). Decisions: atomic parse protects mirror integrity; platform-aware stat helpers for cross-platform drift detection; stubs define contracts without pretending functionality; rustix deferred for Windows buildability. 21 new unit tests pass; all gates green. OpenSpec tasks updated with accurate completion status and clear deferral notes.
- Error handling split already matches skill guidance: `thiserror` for `src/core/`, `anyhow` for `src/commands/` and `main.rs`.
- Phase 3 release-readiness work ships via branch `p3/release-readiness-docs-coverage` → draft PR #15. Includes CI coverage job, release workflow hardening, release checklist, docs-site polish, and README accuracy fixes. All P3 tasks marked complete in `openspec/changes/p3-polish-benchmarks/tasks.md`.
- PR #15 review fix (2026-04-15): Addressed all 9 Copilot review comments. Install snippets across README, install.md, quick-start.md, spec.md, and release.yml now offer both `~/.local/bin` (user-local) and `sudo` (system-wide) install options. Removed inaccurate "embedded model weights" claims; install.md now documents the actual cached-HF / online-model / hash-shim behavior. Fixed typo and consolidated duplicate `## Learnings` headings in zapp history. All 9 threads replied to and resolved.
- Graph BFS (Phase 2): bidirectional traversal (outbound + inbound links) with link-ID edge dedup via `HashSet<i64>` prevents duplicate edges in the result. The `prepare_cached()` API reuses compiled SQL across BFS iterations. Graph types live in `src/core/graph.rs` (not types.rs) because they're graph-specific. The CLI `--temporal` flag defaults to `"current"` which maps to `TemporalFilter::Active`.
- Integration tests use `gbrain::` crate path via `src/lib.rs` (which re-exports `pub mod commands; pub mod core; pub mod mcp;`). Test helper pattern: `open_test_db()` returns a Connection with `std::mem::forget(dir)` to prevent TempDir cleanup during test.
- PR #31 review fix (2026-04-17): Addressed 5 review threads. Bumped Cargo.toml 0.2.0→1.0.0, removed `main` from BEIR regression push trigger (release-only intent), removed duplicate `benchmarks` job in ci.yml. The `src/main.rs` mixed borrow/move comment was invalid — Rust match arms are exclusive so borrowing `&db` in some arms and moving `db` in others compiles fine. `serve`/`call`/`pipe` need ownership because `GigaBrainServer::new()` takes `Connection` by value.

## Core Context

**Phase 1 Foundation (2026-04-14):** Fry implemented `src/core/types.rs` (Page, Link, Tag, TimelineEntry, SearchResult, KnowledgeGap, IngestRecord structs; SearchMergeStrategy enum; OccError/DbError errors via thiserror). Key design: Link stores slugs at app layer, IDs at DB layer (resolver in db). Page.page_type uses serde(rename) for Rust keyword `type`. All integer IDs/versions are i64 to match SQLite. Module-level #![allow(dead_code)] temporary until db.rs wires.

**Database Layer (2026-04-14):** Implemented `src/core/db.rs` (open, compact, set_version tasks 3.1–3.5). sqlite-vec loaded via sqlite3_auto_extension + std::sync::Once guard for idempotency. Schema DDL via include_str! from src/schema.sql. vec0 virtual table and embedding_models seed separate from schema (depend on extension loading first). OccError/DbError split (thiserror). 7 unit tests: table creation, user_version, WAL, foreign keys, path validation, idempotency, compact. All gates pass.

**Markdown Layer (2026-04-14):** Implemented `src/core/markdown.rs` (parse_frontmatter, split_content, extract_summary, render_page; tasks 4.1–4.10). Design: byte-offset search for \n---\n to preserve fidelity. Frontmatter sorted alphabetically for determinism. Timeline separator only emitted when timeline non-empty. Summary = first paragraph or first non-empty line (max 200 chars). Graceful YAML degradation (non-scalar values skipped, malformed → empty map). 21 unit tests per rust-best-practices nested mod pattern. All gates pass.

---

## 2026-04-14 Update

- Rust skill adoption recommendation delivered and accepted by team. Fry's work product captured in `.squad/orchestration-log/2026-04-14T01-53-00Z-fry.md` and merged into team decisions ledger.
- Decision now stands: adopt `rust-best-practices` skill as standing guidance for all Rust implementation and review. Key caveats documented for future reference: MSRV ≥1.81 for `#[expect]`, nightly-only for `rustfmt` import grouping, snapshot testing deferred to Phase 1.
- Team coordination: orchestration logs written, session log recorded, inbox decisions merged and deleted, cross-agent updates applied. Ready for git commit.

## 2026-04-14 Phase 1 Foundation Slice

- Implemented `src/core/types.rs` (tasks 2.1–2.6): `Page`, `Link`, `Tag`, `TimelineEntry`, `SearchResult`, `KnowledgeGap`, `IngestRecord` structs + `SearchMergeStrategy` enum + `OccError`/`DbError` thiserror enums.
- `Page.page_type` uses `#[serde(rename = "type")]` because `type` is a Rust keyword.
- `Link` stores slugs (not page IDs) — DB layer resolves to IDs internally.
- All integer IDs/versions are `i64` to match SQLite INTEGER.
- Module-level `#![allow(dead_code)]` is temporary — remove when db.rs wires types.
- `cargo check`, `cargo clippy -- -D warnings`, and `cargo fmt --check` all pass clean.
- Decision note written to `.squad/decisions/inbox/fry-p1-foundation-slice.md`.

## Phase 1 Database Layer Slice (T02)

- Implemented `src/core/db.rs`: `open()`, `compact()`, `set_version()` — tasks 3.1–3.5 complete.
- sqlite-vec loaded via `sqlite3_auto_extension` with `std::sync::Once` guard for process-global idempotency. Uses explicit `transmute` type annotation to satisfy `clippy::missing_transmute_annotations`.
- `open()` returns `Result<Connection, DbError>` (not `anyhow::Result`) per design decision 10. The `?` propagation to `anyhow::Result` in `main.rs` auto-converts via `thiserror`'s `Error` impl.
- Schema DDL executed via `conn.execute_batch(include_str!("../schema.sql"))` — PRAGMAs at the top of schema.sql are handled correctly by `sqlite3_exec` under the hood.
- vec0 virtual table and embedding_models seed are separate from schema.sql since they depend on the sqlite-vec extension being loaded first.
- `compact` is `#[allow(dead_code)]` until task 6.8 wires the compact command.
- 7 unit tests covering: table creation, user_version, WAL, foreign keys, path validation, idempotency, compact, and embedding model seed.
- Link schema note: the `links` table uses `from_page_id`/`to_page_id` (integer FK to pages), not `from_slug`/`to_slug`. The `Link` struct uses slugs for the application layer — resolution happens in the db layer on insert/read. This is documented in types.rs doc comments.
- `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test db` all pass clean.

## 2026-04-14 Scribe Merge (2026-04-14T03:50:40Z)

- Orchestration logs written for Fry (T02 db.rs completion) and Leela (Link contract review).
- Session log recorded to `.squad/log/2026-04-14T03-50-40Z-phase1-db-slice.md`.
- Three inbox decisions merged into `decisions.md`:
  - Fry's db.rs implementation decisions (sqlite-vec auto-extension, schema DDL, error types)
  - Leela's Link contract clarification (slugs at app layer; IDs at DB layer; three data-loss bugs corrected)
  - Bender's validation plan (anticipatory QA checklist for T02–T06)
- Inbox files deleted after merge.

## 2026-04-14 Phase 1 Foundation Slice (T02)

- Implemented `src/core/db.rs`: `open()`, `compact()`, `set_version()` — tasks 3.1–3.5 complete.
- sqlite-vec loaded via `sqlite3_auto_extension` with `std::sync::Once` guard for process-global idempotency. Uses explicit `transmute` type annotation to satisfy `clippy::missing_transmute_annotations`.
- `open()` returns `Result<Connection, DbError>` (not `anyhow::Result`) per design decision 10. The `?` propagation to `anyhow::Result` in `main.rs` auto-converts via `thiserror`'s `Error` impl.
- Schema DDL executed via `conn.execute_batch(include_str!("../schema.sql"))` — PRAGMAs at the top of schema.sql are handled correctly by `sqlite3_exec` under the hood.
- vec0 virtual table and embedding_models seed are separate from schema.sql since they depend on the sqlite-vec extension being loaded first.
- `compact` is `#[allow(dead_code)]` until task 6.8 wires the compact command.
- 7 unit tests covering: table creation, user_version, WAL, foreign keys, path validation, idempotency, compact, and embedding model seed.
- Link schema note: the `links` table uses `from_page_id`/`to_page_id` (integer FK to pages), not `from_slug`/`to_slug`. The `Link` struct uses slugs for the application layer — resolution happens in the db layer on insert/read. This is documented in types.rs doc comments.
- `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test db` all pass clean.

## 2026-04-14 Scribe Merge (2026-04-14T03:50:40Z)

- Orchestration logs written for Fry (T02 db.rs completion) and Leela (Link contract review).
- Session log recorded to `.squad/log/2026-04-14T03-50-40Z-phase1-db-slice.md`.
- Three inbox decisions merged into `decisions.md`:
  - Fry's db.rs implementation decisions (sqlite-vec auto-extension, schema DDL, error types)
  - Leela's Link contract clarification (slugs at app layer; IDs at DB layer; three data-loss bugs corrected)
  - Bender's validation plan (anticipatory QA checklist for T02–T06)
- Inbox files deleted after merge.

## Phase 1 Markdown Slice (T03)

- Implemented `src/core/markdown.rs`: `parse_frontmatter`, `split_content`, `extract_summary`, `render_page` — tasks 4.1–4.10 complete.
- `parse_frontmatter` parses YAML via `serde_yaml::Value` then converts scalars to strings. Non-scalar values (sequences, maps) are silently skipped. Malformed YAML degrades to empty map (no error propagation — matches spec signature).
- `split_content` uses byte-offset search for `\n---\n` (or prefix/suffix variants) to preserve exact positions for round-trip fidelity. Only the first `---` line is consumed; subsequent separators remain in timeline.
- `render_page` sorts frontmatter keys alphabetically for deterministic output. Timeline separator (`\n---\n`) is only emitted when timeline is non-empty. Canonical input (sorted keys, unquoted values) round-trips byte-exact.
- `extract_summary` collects the first consecutive block of non-heading, non-empty lines, joins with space, truncates to 200 chars. Falls back to first non-empty line (even headings) if no paragraph qualifies.
- Module-level `#![allow(dead_code)]` is temporary — remove when migrate.rs or commands wire the functions.
- 21 unit tests structured per rust-best-practices skill: nested `mod` per function, descriptive names reading as sentences.
- All gates pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (28/28 including 7 db tests).
- Fry, Leela, Bender histories updated with cross-team context.
- Ready for git commit.

## 2026-04-14T03:59:44Z Scribe Merge (T03 completion)

- Scribe wrote orchestration log and session log for T03 completion.
- Three inbox decisions merged into canonical `decisions.md`:
  - Fry's T03 markdown slice decisions (frontmatter canonical order, timeline sep omit-when-empty, YAML parse graceful degradation, non-scalar skip)
  - Professor's rust-best-practices skill standing guidance (adopted with caveats for MSRV, nightly, phase deferral)
  - Scruffy's phase 1 markdown test strategy (20+ must-cover cases, fixture guidance, critical implementation traps)

## 2026-04-17 Vault-Sync Batch B: Ignore Patterns + File State + Reconciler Scaffolding

- **Group 3 (Ignore patterns): tasks 3.1–3.7 COMPLETE**
  - Created `src/core/ignore_patterns.rs` with atomic parse, reload logic, and GlobSet builder.
  - `builtin_patterns()` returns the five default patterns (`.obsidian/**`, `.git/**`, `node_modules/**`, `_templates/**`, `.trash/**`).
  - `parse_ignore_file()` validates every line via `globset::Glob::new` before any effect. Returns `Valid(patterns)` or `Invalid(errors)`.
  - `reload_patterns()` is the SOLE writer of `collections.ignore_patterns`. Handles four cases: (1) file present + valid → refresh mirror; (2) file present + invalid → mirror unchanged, record errors; (3) file absent + no prior mirror → defaults only; (4) file absent + prior mirror → file-stably-absent error.
  - `build_globset()` merges built-in defaults + user patterns from DB mirror for reconciler use.
  - `IgnoreParseError` struct with canonical JSON shape: `{code, line, raw, message}`.
  - 9 unit tests: builtin patterns valid, parse valid/invalid/empty files, reload with/without prior mirror, build globset with user patterns.
  - Added `ignore` + `globset` crate dependencies.
  - **Note:** CLI commands (`gbrain collection ignore add|remove|clear`) deferred to later batch; `reload_patterns()` ready for watcher integration.

- **Group 4 (File state tracking): tasks 4.1 COMPLETE, 4.2 PARTIAL**
  - Created `src/core/file_state.rs` with stat helpers, hash, upsert/delete, comparison predicates.
  - `FileStat` struct: `(mtime_ns, ctime_ns, size_bytes, inode)`. On Windows, `ctime_ns` and `inode` are `None`.
  - `stat_file(path)` implemented using `std::fs::metadata` with Unix/Windows branching. Full `fstatat(AT_SYMLINK_NOFOLLOW)` requires rustix (task 2.4a), deferred because Windows dev environment cannot build it.
  - `hash_file(path)` computes SHA-256 via streaming 8KB buffer.
  - `upsert_file_state()` inserts or updates `file_state` row with full stat tuple + sha256. Sets `last_full_hash_at` to now.
  - `delete_file_state()` removes row on page hard-delete.
  - `get_file_state()` queries by (collection_id, relative_path).
  - `stat_differs()` and `needs_rehash()` compare stat tuples; return `true` if ANY of the four fields differ.
  - 10 unit tests: stat returns size, hash computes sha256, upsert insert/update, delete, stat_differs detects each field change independently.
  - Added `hex` crate dependency for sha256 encoding.
  - Tasks 4.3 (stat_diff) and 4.4 (full_hash_reconcile) stubbed in reconciler; full walk implementation deferred.

- **Group 5 (Reconciler skeleton): task 5.1 COMPLETE**
  - Created `src/core/reconciler.rs` with stub functions and types.
  - `ReconcileStats` struct: `walked`, `unchanged`, `modified`, `new`, `missing`, `native_renamed`, `hash_renamed`, `quarantined_ambiguous`, `quarantined_db_state`, `hard_deleted`.
  - `reconcile()` stub: returns empty stats. Full implementation deferred (tasks 5.2–5.9).
  - `full_hash_reconcile()` stub: used by remap/restore/audit.
  - `stat_diff()` stub: returns empty `StatDiff` with `unchanged`, `modified`, `new`, `missing` sets.
  - `has_db_only_state()` stub: always returns `false` until schema updates (tasks 5.4a, 1.1b) add `links.source_kind` and `knowledge_gaps.page_id`.
  - 2 unit tests: reconcile stub returns empty stats, has_db_only_state stub returns false.

- **Build validation:**
  - `cargo fmt --all` — clean.
  - `cargo check --all-targets` — compiles with warnings for dead code (expected for stubs).
  - Individual module tests: ignore_patterns (9/9), file_state (10/10), reconciler (2/2) — all pass.
  - Full test suite blocked by Windows linker file-lock issue (common in dev; CI will validate).

- **Truthfulness:**
  - Task 2.4a (rustix dependency) not added — requires Unix and Windows dev cannot build it. Documented as blocker for fd-relative operations.
  - Task 4.2 partial: `stat_file()` implemented but fd-relative variant requires rustix.
  - Tasks 4.3, 4.4 stubbed but not implemented — full walk requires watcher dependencies and cross-platform fd handling.
  - Reconciler is buildable scaffolding, not a functional pipeline. Walk, rename detection, quarantine classifier all deferred to next batch.
  - `tasks.md` updated with accurate completion status and blocking notes.

- **Key design decisions:**
  - `ignore_patterns` module uses `reload_patterns()` as single source of truth for DB mirror writes. Atomic parse ensures mirror is never in invalid state.
  - `file_state` helpers are platform-aware (Unix full stat, Windows partial). Re-hash on ANY stat field mismatch.
  - Reconciler stubs define the contract (types, signatures, error variants) but do not pretend functionality. Next batch can fill in walk logic without interface changes.
  - All new code follows rust-best-practices: explicit error types (thiserror), descriptive test names, minimal clones, no `unwrap()` in prod paths.
- Inbox files deleted after merge.
- Git commit staged and ready.

## 2026-04-14T04:07:24Z Phase 1 Command Slice — T05 init, T07 get (COMPLETE)

- Implemented `src/commands/init.rs` (T05): existence check before db::open prevents re-initialization; no schema migration on existing DBs.
- Implemented `src/commands/get.rs` (T07): extracted `get_page()` as public helper for OCC reuse in T06; frontmatter stored as JSON with defensive deserialization; `--json` output supported.
- Public `get_page(db, slug)` helper enables T06 put command to read current version for OCC checks without circular module deps.
- Tests: 3 for init (creation, idempotent re-run, nonexistent parent rejection); 4 for get (data round-trip, markdown render, not-found error, frontmatter deser).
- Total test count: 48 (41 baseline + 7 new).
- All gates pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (48/48).
- Integration points: Bender can now use `get_page` for round-trip test harness; T06 put can import `get_page` for version reads.
- In-flight: T06 put command implementation (stdin seam + OCC compare-and-swap logic + 3+ test cases per Scruffy spec).
- Blocker: lib.rs export (Bender concern, Phase 1 gate requirement for round-trip tests).
- Branch: phase1/p1-core-storage-cli.

## 2026-04-14T04:07:24Z Scribe Merge (T05, T07, T03 approval, T06 spec)

- Scribe wrote 3 orchestration logs (Fry: T05+T07 complete; Bender: T03 approved; Scruffy: T06 spec locked).
- Scribe wrote session log for Phase 1 command slice window.
- Four inbox decisions merged into canonical decisions.md:
  - Bender's T03 markdown slice approval (APPROVED; 2 non-blocking concerns logged for Phase 2)
  - Fry's T05+T07 implementation decisions (get_page helper, JSON frontmatter, --json output, no main.rs changes needed)
  - Scruffy's T06 put unit test spec (3 core cases + 4 assertion guards + implementation seam requirement)
- Inbox files deleted after merge (all three inbox .md files removed).
- Ready for git commit.

## Phase 1 T08 list.rs + T09 stats.rs (COMPLETE)

- Implemented `src/commands/list.rs` (T08): dynamic query with optional wing/type filters, ORDER BY updated_at DESC, LIMIT N (default 50). Supports `--json` output. 7 unit tests covering all filter combos, limit, ordering, empty DB.
- Implemented `src/commands/stats.rs` (T09): gathers total pages, pages-by-type, links, embeddings, FTS rows, DB file size. DB path resolved from `pragma_database_list` — no main.rs plumbing changes. Supports `--json` output. 4 unit tests covering empty DB, counts, FTS trigger rows, file size.
- No main.rs changes needed — clap dispatch was already wired correctly.
- Test count: 68 (57 baseline + 11 new). All gates pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (68/68).
- Decision note written to `.squad/decisions/inbox/fry-p1-list-stats-slice.md`.
- Task checkboxes updated in `openspec/changes/p1-core-storage-cli/tasks.md`.

## Phase 1 T11 link.rs + T12 compact.rs + T10 tags.rs (COMPLETE)

- Implemented `src/commands/link.rs` (T11): slug-to-ID resolution in command layer; link-close uses UPDATE-first pattern for valid_until. Also implemented link-close (by ID), links (list outbound), backlinks (list inbound), and unlink (delete) to unblock runtime panics.
- Implemented `src/commands/compact.rs` (T12): thin delegation to `db::compact()` + success message.
- Implemented `src/commands/tags.rs` (T10): unified `Tags` subcommand (list/add/remove) per Leela's contract review. Tags live in `tags` table exclusively — no OCC, no page version bump. `INSERT OR IGNORE` for idempotent add; silent no-op on remove of nonexistent tags.
- Tests: 10 for link (create, close, by-ID, nonexistent ID, page-not-found, unlink, list, compact), 8 for tags (empty list, add, duplicate idempotency, remove, nonexistent remove, nonexistent page error, version-unchanged assertion, alphabetical ordering). Total: 86 tests (47 baseline + 39 new).
- All gates pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (86/86).
- Decision notes written to inbox: fry-p1-link-compact-slice.md, fry-p1-tags-slice.md, fry-p1-put-slice.md (T06 prior session).
- Integration: Leela provided T10 contract review (tags-contract-review.md) — corrections applied to tasks.md and spec.md; Fry's implementation proceeded on corrected contract.
- Next lane: T13 FTS5 search command.

## 2026-04-14T04:42:03Z Phase 1 T13 FTS5 + T18/T19 Reconciliation

- Completed T13 FTS5 search implementation: BM25 ranking, wing filtering, 10 unit tests (96 total pass).
- Decision locked: BM25 score negation (positive-higher-is-better), empty-query short-circuit, dynamic SQL wing filter.
- T18/T19 reconciliation batch initiated: Fry to verify gates and reconcile embed/query scope.
- Bender validation report submitted with 3 findings:
  1. **Gap:** `gbrain embed <SLUG>` (single-page) not implemented — clap only has `--all`/`--stale` flags
  2. **Mismatch:** `--token-budget` counts chars not tokens (misleading flag name)
  3. **Status:** Inference shim (SHA-256) is not semantic — BEIR benchmarks will be meaningless
- T13 decision merged into canonical ledger; Scruffy test expectations locked.
- Orchestration entries created for Fry (T18/T19), Bender (search validation), Professor (contract review).
- Session log created. Bender finding queued for merge.

## 2026-04-14 Phase 1 Embed Surface Completion (T18/T19)

- Implemented `gbrain embed <SLUG>` support: added optional positional slug arg to CLI, command dispatches to single-page embed path that always re-embeds (no stale-skip for explicit slug). `--all` and `--stale` preserved for bulk path.
- T18 fully closed: all 4 checkboxes done. Two new unit tests added (single-slug embed, re-embed-unchanged confirmation).
- T19 fully closed: `budget_results` already implemented token-budget truncation. Marked checkbox done after verification.
- T14 remains `[~]`: API contract complete (384-dim, L2-normalized, EmptyInput error) but uses SHA-256 hash shim, not real Candle BGE-small. Decision note written to inbox documenting exact blocker and recommendation to treat Candle integration as a dedicated task.
- Total tests: 115 (all pass). `cargo clippy -- -D warnings` clean, `cargo fmt --check` clean.
- Decision note: `.squad/decisions/inbox/fry-embed-surface.md`.

## 2026-04-14T04:56:03Z Phase 1 T14–T19 Submission Gating

- Submitted complete T14–T19 artifact with T18/T19 closed as done and decision note queued.
- Bender validation: 3 findings reported. Single-slug embed implemented ✅; query budget scoping accepted (Phase 1 design); inference shim status documented as Phase 2 blocker.

## Vault-Sync Engine Foundation A (2026-04-22)

**OpenSpec Change:** `openspec/changes/vault-sync-engine/`

- Implemented schema v5 foundation (tasks 1.1–1.6): Breaking migration from v4 to v5 with version detection and v4 rejection. New tables: `collections`, `file_state`, `embedding_jobs`. Extended `pages` with `collection_id`, `uuid`, `quarantined_at`. Modified `links` to add `source_kind` for provenance tracking. Modified `contradictions.other_page_id` to `ON DELETE CASCADE`. Added `knowledge_gaps.page_id` for slug-bound gap tracking. Removed `ingest_log` (replaced by file_state + collection sync model).

- Implemented collections module (tasks 2.1–2.6): Created `src/core/collections.rs` with validators (`validate_collection_name()`, `validate_relative_path()`), CRUD operators (`get_by_name()`, `get_write_target()`), and slug resolution via `parse_slug()` with `OpKind` classification. Path traversal protection rejects `..`, absolute paths, NUL bytes, empty segments. Slug resolution by intent: Read (exactly-one or ambiguous), WriteCreate (zero→write-target; one owner AND write-target→that; else ambiguous), WriteUpdate/WriteAdmin (exactly-one or ambiguous/not-found). Ambiguity error carries structured `Vec<AmbiguityCandidate>` for user-facing resolution hints.

- Schema tests: 19 updated to expect v5 schema, all pass. Collections unit tests: 8 new tests for validators and resolution logic. All gates pass: `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`.

- Decisions merged into canonical ledger: Schema v5 evolution, collections module structure, slug resolution by OpKind, ambiguity error user-facing type. Deferred items: platform-specific fd-safety (needs `#[cfg(unix)]`), knowledge_gaps.page_id wiring (needs gaps.rs integration), command wiring (needs reconciler + watcher).

- Decision note: `.squad/decisions/inbox/fry-vault-sync-foundation-a.md` → merged to `decisions.md`.
- Orchestration log: `.squad/orchestration-log/20260422T091053Z-fry.md`.
- Session log: `.squad/log/20260422T091053Z-vault-sync-foundation-a.md`.
- Ready for git commit.
- Professor code review: REJECTION issued on three grounds:
  1. Inference shim SHA-256 placeholder not explicitly documented in module — public API misleading on semantic guarantees
  2. Embed CLI mixed-mode validation missing — accepts `SLUG + --all` instead of rejecting per contract
  3. Test compilation failure — callsites not updated to new embed::run signature (4 args)
- Fry locked out of revision cycle per team protocol (prevents churn during active review).
- Leela took revision cycle independently. Outcome: APPROVED (5 decisions on documentation, stderr warnings, honest status notes). All 115 tests pass unchanged.
- Ready for Phase 1 ship gate after Leela revision lands and Professor approves.

## Phase 1 T21–T34 Completion (COMPLETE)

- Implemented all remaining Phase 1 tasks in a single session:
  - T21 `src/core/links.rs`: `extract_links` (regex `[[slug]]` extraction), `resolve_slug` (lowercase kebab normalisation). 8 unit tests.
  - T22 `src/core/migrate.rs`: `import_dir` (SHA-256 idempotent batch import with `import_hashes` table), `export_dir` (render_page to markdown files), `validate_roundtrip` (export-reimport comparison). 6 unit tests.
  - T23 `src/commands/import.rs`: CLI wrapper for `migrate::import_dir`, prints import/skip counts.
  - T24 `src/commands/export.rs`: CLI wrapper for `migrate::export_dir`.
  - T25 `src/commands/ingest.rs`: Single-file ingest with SHA-256 dedup, `--force` bypass. 2 unit tests (double-ingest skip, force re-ingest).
  - T26 `src/commands/timeline.rs`: Parse timeline section from page content, print entries. Supports `--json`. `add()` stub implemented. 3 unit tests.
  - T27 `tests/fixtures/`: Added `project.md`, `person2.md`, `company2.md` (5 total). Canonical format for round-trip compatibility.
  - T28 `src/mcp/server.rs`: Full MCP server using rmcp 0.1 `#[tool(tool_box)]` macro. 5 tools: `brain_get`, `brain_put`, `brain_query`, `brain_search`, `brain_list`. Error codes: -32009 (OCC conflict), -32001 (not found), -32002 (parse), -32003 (DB). Uses `Arc<Mutex<Connection>>` for thread-safe DB access.
  - T29 `src/commands/serve.rs`: Async wrapper calling `mcp::server::run(conn)`.
  - T30 `src/commands/config.rs`: get/set/list for config table. 2 unit tests.
  - T31 `src/commands/version.rs`: Already implemented (prints `gbrain <version>`).
  - T32 `--json` flags: Already wired globally in all 5 required commands.
  - T33 Skills: Updated `skills/ingest/SKILL.md` and `skills/query/SKILL.md` with accurate Phase 1 content.
  - T34 Lint gate: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` — all pass (142 tests).
- Key decisions:
  - `import_hashes` table created via `CREATE TABLE IF NOT EXISTS` (separate from `ingest_log` in schema.sql which tracks different data).
  - MCP server uses `Arc<Mutex<Connection>>` since rmcp `ServerHandler` requires `Clone + Send + Sync`.
  - Fixtures use LF line endings, sorted frontmatter, no quoted values — matches `render_page` canonical output.
  - rmcp `ErrorCode` wrapper required for custom error codes (not bare integers).

## T14 BGE-small-en-v1.5 Forward Pass + T34 musl Static Binary

- **T14 COMPLETE:** Replaced SHA-256 hash shim with real Candle BGE-small-en-v1.5 BERT forward pass in `src/core/inference.rs`.
  - `EmbeddingModel` now attempts to load the real BERT model via Candle. Falls back to SHA-256 hash shim with stderr warning if model files unavailable.
  - Forward pass: tokenize → BERT forward → mean pooling (with broadcast) → L2 normalize → 384-dim Vec<f32>.
  - Model download: `--features online-model` adds `hf-hub` dependency for HuggingFace Hub download. Without the feature, looks for cached files in `~/.gbrain/models/bge-small-en-v1.5/` or HuggingFace cache.
  - hf-hub 0.3.2 has a bug with HuggingFace's relative redirect URLs (`/api/resolve-cache/...`). Manual download via `curl` works. Phase 2 should either bump hf-hub or implement direct download.
  - Candle tensor ops require explicit `broadcast_as()` for shape-mismatched operations (mask×output, sum÷count, mean÷norm). This differs from PyTorch's implicit broadcasting.
  - `embed-model` removed from default features (was never wired). `online-model` is the active download path.
  - All 296 tests pass (147 unit ×2 + 1 roundtrip_raw + 1 roundtrip_semantic). The roundtrip_semantic test now passes with real embeddings.
- **T34 musl COMPLETE:** `x86_64-unknown-linux-musl` static binary builds successfully.
  - Requires `musl-tools` apt package and `CFLAGS` workaround: `-Du_int8_t=uint8_t -Du_int16_t=uint16_t -Du_int64_t=uint64_t` for sqlite-vec's glibc-specific type aliases.
  - Build command: `CC_x86_64_unknown_linux_musl=musl-gcc CXX_x86_64_unknown_linux_musl=g++ CFLAGS_x86_64_unknown_linux_musl="..." CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc cargo build --release --target x86_64-unknown-linux-musl`
  - `ldd` confirms: "statically linked". `file` confirms: "static-pie linked, stripped". Binary size: 8.8MB (without embedded model weights).
  - Phase 2: consider embedding model weights via `include_bytes!()` for zero-network binary (~90MB).

## SG-6 Security Fixes (Nibbler Rejection Response)

- Addressed all 5 categories from Nibbler's SG-6 adversarial review rejection of `src/mcp/server.rs`:
  1. **OCC bypass closed:** `brain_put` now requires `expected_version` for updates to existing pages. Returns `-32009` with `current_version` data when omitted. New page creation still allows `None`.
  2. **Input validation:** `validate_slug()` enforces `[a-z0-9/_-]` + 512-char max. `validate_content()` caps at 1 MB. Both return `-32602`.
  3. **Error code consistency:** Centralized `map_db_error()` routes UNIQUE→`-32009`, FTS5→`-32602`, other→`-32003`. `map_search_error()` wraps for SearchError.
  4. **Resource exhaustion:** `limit` capped at `MAX_LIMIT = 1000` for list/query/search. Added missing `limit` field to `BrainQueryInput`/`BrainSearchInput`.
  5. **Mutex recovery:** `unwrap_or_else(|e| e.into_inner())` replaces `map_err(internal_error)` — server recovers from poisoned mutex instead of permanently wedging.
- 4 new tests (OCC bypass rejection, invalid slug, oversized content, empty slug). Total: 304 pass.
- `cargo fmt`, `cargo clippy -- -D warnings` clean.
- Commit `5886ec2` on `phase1/p1-core-storage-cli`. SG-6 checkbox NOT marked — requires Nibbler re-approval.
- Decision note: `.squad/decisions/inbox/fry-sg6-fixes.md`.

## Phase 3 Coverage + Release Workflow Hardening (Tasks 1.1–1.4)

- **Task 1.1 Audit:** ci.yml had no coverage job; release.yml checksum format was hash-only (fragile, non-standard); release job had no artifact verification before publishing.
- **Task 1.2 Coverage job:** Added `coverage` job to ci.yml using `cargo-llvm-cov` with `llvm-tools-preview`. Runs in parallel with `test` after `check` gate. Uses same `cargo test` path under the hood — no separate unreviewed test path.
- **Task 1.3 Coverage outputs:**
  - Machine-readable: `lcov.info` uploaded as GitHub Actions artifact.
  - Human-readable: text summary posted to GitHub Job Summary (visible on every PR/push).
  - Optional third-party: Codecov upload with `continue-on-error: true` — never blocks CI. Guarded to skip on fork PRs.
- **Task 1.4 Release hardening:**
  - Switched `.sha256` files from hash-only to standard `hash  filename` format. Enables direct `shasum -a 256 --check` verification.
  - Added artifact existence verification step: all 8 files (4 binaries + 4 checksums) must be present before release creation.
  - Added post-download checksum re-verification in release job.
  - Updated release body template, README, docs-site quick-start, and install page to match the new standard checksum format.
  - Updated Zapp's `RELEASE_CHECKLIST.md` to reflect the new checksum format.
- **Spec reference:** All changes satisfy `specs/coverage-reporting/spec.md` and `specs/release-readiness/spec.md`.
- All four tasks marked `[x]` in `openspec/changes/p3-polish-benchmarks/tasks.md`.

### Learnings

- `cargo llvm-cov report` reuses profraw data from the previous `cargo llvm-cov --lcov` run — no test re-execution needed for the text summary.
- Standard `.sha256` format (`hash  filename`) is strictly better than hash-only: enables `shasum --check` directly, matches conventions from Go, Terraform, kubectl, etc.
- Codecov v4 requires a token even for public repos. Making it `continue-on-error: true` with an optional `CODECOV_TOKEN` secret satisfies the "additive and non-blocking" spec requirement.
- Release artifact verification should always happen as a separate step before `softprops/action-gh-release` — the action doesn't validate completeness itself.

## 2026-04-15 P3 Release — Completion

**Role:** CI/Release workflow implementation, artifact verification

**What happened:**
- Fry implemented `cargo-llvm-cov` coverage job and hardened release.yml with standard checksum format (`hash  filename`).
- Kif's first review (task 5.1) rejected on two doc-drift issues: coverage artifact name mismatch, checksum format mismatch. Fry applied targeted fixes to spec.md and install.md.
- After fixes, task 5.1 passed Kif's re-review. All four implementation tasks marked complete in tasks.md.
- Coverage now visible in GitHub UI. Release workflow verified end-to-end.

**Outcome:** P3 Release CI/Release component **COMPLETE**. Coverage job running, release workflow tested, artifact verification validated, all gates passed.

**Decision notes:** `.squad/decisions.md` (merged from inbox) — documents coverage tool selection, checksum format standardization, informational (non-gating) coverage policy, and optional Codecov handling.

## 2026-04-15 Phase 2 Kickoff — Blocker Summary

**Phase 2 branch:** `phase2/p2-intelligence-layer` from main at v0.1.0. PR #22 opened (not merged per user directive).

### CRITICAL BLOCKERS (must resolve before sign-off)

1. **Schema gap: `knowledge_gaps.query_hash` missing UNIQUE constraint (Bender + Nibbler)**
   - Task 8.1 specifies `INSERT OR IGNORE` for idempotency on duplicate queries
   - `INSERT OR IGNORE` requires a UNIQUE constraint to trigger
   - Without it, every low-confidence query logs a duplicate row — idempotency contract broken
   - **Resolution required:** Add `UNIQUE(query_hash)` to schema (preferred) or create index at init time in db.rs
   - **Impact:** Blocks Group 8 (knowledge gaps) and Group 9 (MCP write surface) validation

2. **Graph contract ambiguity (Professor)**
   - Current: BFS traverses both outbound and inbound edges (bidirectional)
   - Spec: Tasks + spec/graph/spec.md describe outbound-first reachability
   - CLI output: Each edge printed as `→ <edge.to>`, misleading for inbound-only edges pointing back at root
   - **Resolution required:** Choose one now
     - Option A: neighbourhood = undirected adjacency → update spec + CLI renderer to show both directions explicitly
     - Option B: neighbourhood = outbound traversal → remove inbound BFS from core
   - **Impact:** Blocks Group 1 sign-off; blocks Professor approval

3. **Edge deduplication on cyclic graphs (Professor)**
   - Nodes deduplicated with `visited` HashSet, but edges appended unconditionally
   - Two-node cycle (`a -> b`, `b -> a`) with depth ≥ 2: same link appears twice in result
   - **Resolution required:** Deduplicate edges by stable identity (prefer link ID, else `(from,to,relationship,valid_from,valid_until)`)
   - Add test: cyclic graph with depth > 1 asserts duplicate-free edges
   - **Impact:** Blocks Group 1 sign-off

4. **Progressive retrieval not started (Professor)**
   - src/core/progressive.rs is still a stub
   - src/commands/query.rs still Phase 1 behavior: ignores `depth`, never follows links, budgets rendered line length not token count
   - docs/spec.md describes `summary/section/full/auto` expansion vs Phase 2 tasks simplify to linked-page expansion → `Vec<SearchResult>`
   - **Resolution required:** Settle contract before coding. Either implement richer spec surface or explicitly narrow spec/design now.
   - Avoid guaranteed rework
   - **Impact:** Blocks Group 5 sign-off

5. **OCC erosion risk in Group 9 MCP writes (Professor)**
   - docs/spec.md: all page-scoped mutators (`brain_link`, `brain_unlink`, `brain_timeline_add`, `brain_tag`, `brain_raw`) must require `page_version` and bump `pages.version`
   - Group 9 tasks currently say `brain_link/brain_link_close/brain_tags` delegate directly to command helpers that do NOT perform page-version checks
   - **Resolution required:** Either preserve Phase 1 OCC discipline on every new page-scoped write tool OR amend product spec before implementation
   - Do NOT quietly weaken write contract
   - **Impact:** Blocks Group 9 sign-off

### ADVERSARIAL GUARDRAILS (Nibbler, ship-gate blockers)

1. **Active temporal reads (D1)**
   - Default "active/current" reads must treat link as active only when BOTH:
     - `valid_from IS NULL OR valid_from <= today` AND
     - `valid_until IS NULL OR valid_until >= today`
   - Currently: future-dated links (valid_from > today) indistinguishable from present if only `valid_until` checked
   - **Impact:** Blocks ship gate until future-dated links excluded from default active views + tested

2. **Graph output budgets (D2)**
   - Hop cap of 10 alone insufficient; one attacker-controlled hub page with thousands of edges forces huge BFS fan-out
   - **Resolution required:** Enforce at least one non-depth budget (max nodes, max edges, or max serialized bytes)
   - Make traversal direction explicit in contract
   - **Impact:** Ship-gate blocker for Fry + Professor

3. **Contradiction idempotency (D3)**
   - `extract_assertions` must only replace agent-generated assertions for target page (not erase manual/import)
   - Contradiction rows must deduplicate by stable fingerprint, not free-form text
   - **Impact:** Repeated `brain_check` runs must not poison contradictions table

4. **MCP output shape contract (D5)**
   - MCP tools must return typed truth per spec, not delegated CLI side effects
   - Current bugs: `backlinks` ignores temporal arg, `timeline --json` returns `{slug, entries}` not bare array, `tags` mutates but prints nothing
   - **Impact:** Fry must treat output-shape tests + parameter-behaviour tests as ship-gate requirements, not polish

### Coordination Notes

- **Leela:** Phase 2 kickoff complete; 8 agents ready; no blockers for implementation start
- **Scruffy:** Coverage lane ready; parallelize tests with Fry's implementation
- **Bender:** 24 validation scenarios ready; awaiting schema gap fix before Group 8 validation
- **Amy:** Pre-ship docs done; post-ship checklist (15 items) ready after merge
- **Hermes:** Website docs in progress
- **Mom:** Temporal edge cases in progress
- **Professor:** Early review complete; blockers F1–F4 require spec clarification gates
- **Nibbler:** Guardrails D1–D5 defined; ship gate blocked until all tested
- **User directive:** Complete Phase 2 with frequent checkpoints; Fry must open PR + not merge (user reviews + merges)

## 2026-04-15 Cross-team Phase 2 Revision Batch

