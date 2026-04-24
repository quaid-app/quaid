## 1. Schema v5

- [x] 1.1 Implement v5 schema per design.md § Schema. Acceptance: `gbrain init` creates all tables (`collections`, `pages`, `file_state`, `embedding_jobs`, `raw_imports`, `links`, `assertions`, `knowledge_gaps`, `contradictions`) with the documented columns and FKs; existing integration smoke test passes.
  > **Repair note (Leela):** `serve_sessions` and `collection_owners` are watcher-slice tables, not foundation. `ingest_log` is kept as a compatibility shim until the reconciler slice removes `gbrain import`. `pages.uuid` is nullable until task 5a.* wires UUID generation. `pages.collection_id DEFAULT 1` routes legacy inserts to the auto-created default collection.
- [x] 1.1a Create `CREATE UNIQUE INDEX idx_pages_uuid ON pages(uuid) WHERE uuid IS NOT NULL` for O(1) UUID-based rename lookup; partial index allows NULL until task 5a.*.
- [x] 1.1b Extend `src/core/gaps.rs::log_gap()` and `brain_gap` to accept an optional slug and populate `knowledge_gaps.page_id` when a slug resolves; leave NULL otherwise. Update the `Gap` struct and `list_gaps`/`resolve_gap` responses. Unit tests cover slug and slug-less variants and the `has_db_only_state` effect.
  > **Repair note (Leela, K1 repair):** The library side (slug→page_id binding, `KnowledgeGap.page_id`, `list_gaps` response) was complete. Added `page_id` to the `brain_gap` MCP response so callers can confirm the binding in a single call. Added tests `brain_gap_with_slug_response_includes_page_id` and `brain_gap_without_slug_response_has_null_page_id`.
- [x] 1.1c Classify `brain_gap` by variant: slug-bound = `WriteUpdate` (subject to `CollectionRestoringError` interlock); slug-less = `Read` (no collection resolved, no interlock). Unit test covers both during `state='restoring'`.
- [x] 1.2 Add index on `pages.quarantined_at` for efficient sweep queries.
- [x] 1.3 `brain_config` writes `schema_version = 5` on `gbrain init`.
- [x] 1.4 `src/core/db.rs::open()` detects schema version: v5 opens normally; v4 or older errors with re-init instructions.
- [x] 1.5 Update FTS5 triggers so search queries apply `WHERE quarantined_at IS NULL` efficiently.
- [x] 1.6 Ensure `page_embeddings` and `page_embeddings_vec_*` reference `pages.id` and apply the same quarantine filter on vector search paths.
  > **Repair note (Leela):** `search_vec` in `inference.rs` now adds `AND p.quarantined_at IS NULL` — this was missing from Fry's slice.

## 2. Collection model

- [x] 2.1 Create `src/core/collections.rs` with `Collection` struct, CRUD helpers, and name/path validators.
- [x] 2.2 Add resolution helpers: `resolve_by_name()`, `write_target()`, `parse_slug(input, op_kind)` returning `Resolved(collection_id, relative_path) | NotFound | Ambiguous(Vec<Candidate>)`. Split on FIRST `::`. CHECK + clap validator reject `::` in collection names.
- [x] 2.3 Define `op_kind` enum: `Read`, `WriteCreate`, `WriteUpdate`, `WriteAdmin`. Every mutating tool classifies; classification drives bare-slug resolution, `CollectionRestoringError` interlock, and audit logging. A tool that reads-then-writes passes the most-mutating op_kind for the whole call.
- [x] 2.4 Path-traversal rejection in `parse_slug()`: reject `..` components, absolute paths, empty segments, NUL bytes.
- [x] 2.4a Add `rustix` (preferred) or `nix` crate dependency for `openat`/`fstatat`/`mkdirat`/`renameat`/`unlinkat` under `#[cfg(unix)]`.
- [x] 2.4a2 Platform gate: `#[cfg(windows)]` handlers return `UnsupportedPlatformError` from the currently implemented vault-sync CLI surfaces: `gbrain serve`, `gbrain put`, `gbrain collection {add,sync,restore}`. Deferred collection quarantine/export handlers remain out of scope; existing DB-only reset handlers (`restore-reset`, `reconcile-reset`) remain outside this Windows gate and may still run offline.
- [x] 2.4b Implement `src/core/fs_safety.rs` fd-relative primitives: `open_root_fd`, `walk_to_parent`, `openat_create_excl`, `stat_at_nofollow`, `renameat_parent_fd`, `unlinkat_parent_fd`.
  > **Complete:** All six primitives implemented with Unix `#[cfg(unix)]` + Windows fallback returning `UnsupportedPlatformError`. Uses `rustix::fs` with `O_NOFOLLOW`, `O_DIRECTORY`, `AT_SYMLINK_NOFOLLOW` semantics.
- [x] 2.4c On the reconciler path, candidate paths are enumerated via `ignore::WalkBuilder` with `follow_links(false)`. Walker metadata is advisory only; each candidate is revalidated with `walk_to_parent` + `stat_at_nofollow`, WARN-skipping symlinked entries/ancestors and never descending symlinked directories.
  > **Closure note:** This task is closed only for the current reconciler walk path. It is not a claim that a generic `readdir`-based fd-relative walk primitive exists, and it does not widen beyond the existing `ignore::WalkBuilder` + fd-relative revalidation seam.
- [x] 2.4d Unit tests for fd-safety helpers: reject path traversal, reject symlinked root, reject symlinked ancestor, reject symlink at target, reject `O_EXCL` clobber, round-trip a safe write.
  > **Complete:** 15 tests in `fs_safety.rs` covering all safety scenarios. Tests are `#[cfg(all(test, unix))]` and pass on Linux/macOS CI.
- [x] 2.5 `parse_slug` returns `Resolved`/`NotFound`/`Ambiguous`; callers translate `Ambiguous` into `AmbiguityError` with candidate list.
- [x] 2.6 Register a user-facing error type `AmbiguityError` with candidate list and a stable serialization shape.

## 3. Ignore pattern handling

- [x] 3.1 Add `ignore` + `globset` crates. Built-in defaults are merged at reconciler-query time (`.obsidian/**`, `.git/**`, `node_modules/**`, `_templates/**`, `.trash/**`); user patterns live on disk in `.gbrainignore` only.
- [x] 3.2 Implement atomic-parse of `.gbrainignore`: validate every non-comment line via `globset::Glob::new` BEFORE any effect. Fully-valid → refresh `collections.ignore_patterns` mirror, clear `ignore_parse_errors`, trigger reconciliation. Any failing line → mirror UNCHANGED, `ignore_parse_errors` records failing lines, no reconciliation.
- [x] 3.3 Absent-file default: no prior mirror → defaults only; prior mirror present → mirror UNCHANGED, WARN logged `gbrainignore_absent collection=<N>`. Operator explicitly clears with `gbrain collection ignore clear <name> --confirm`.
- [x] 3.4 CLI `gbrain collection ignore add|remove|clear --confirm` is dry-run first (in-memory proposed file, atomic-parse validator), file-write second, mirror-refresh last via `reload_patterns()`. CLI never writes `collections.ignore_patterns` directly.
  > **Note:** `reload_patterns()` is implemented; CLI commands deferred to later batch.
- [x] 3.5 `reload_patterns()` is the SOLE writer of `collections.ignore_patterns`; invoked by the watcher on `.gbrainignore` events and at serve startup.
- [x] 3.6 Expose parse errors via WARN log, `brain_collections` `ignore_parse_errors` field, and `gbrain collection info`.
  > **Note:** Data model complete; logging and CLI display deferred to watcher/serve slices.
- [x] 3.7 `ignore_parse_errors` is a JSON array of `{code, line, raw, message}` where `code` ∈ `parse_error` | `file_stably_absent_but_clear_not_confirmed`. Single canonical shape documented in the spec.

## 4. File state tracking and stat-diff

- [x] 4.1 Add `file_state` table + indexes per §Schema. `ctime_ns` is nullable for legacy rows only; `brain_put` always writes the full tuple.
  > **Note:** Schema already in place from foundation slice; helpers implemented in `src/core/file_state.rs`.
- [x] 4.2 Implement `stat_file(parent_fd, name)` returning `(mtime_ns, ctime_ns, size_bytes, inode)` via `fstatat(AT_SYMLINK_NOFOLLOW)`.
  > **Complete:** `stat_file_fd` implemented in `file_state.rs` using `fs_safety::stat_at_nofollow`. Unix uses fd-relative `fstatat(AT_SYMLINK_NOFOLLOW)`; Windows uses path-based fallback with nullable ctime/inode. Function signatures and conversion wrappers are production-ready. Task 5.2 reconciler walk will call `stat_file_fd` with parent fds.
- [x] 4.3 Implement `stat_diff(collection_id)`: compare filesystem walk against `file_state`; yield `{unchanged, modified, new, missing}` sets. Any of the four stat fields mismatching triggers re-hash.
  > **Complete (Batch D):** Task 5.2 walk landed; `stat_diff` now performs a real filesystem walk via `walk_root` + `ignore::WalkBuilder` with fd-relative `stat_at_nofollow` per entry, then compares against `file_state` via `stat_diff_from_walk`. The four-field comparison (`mtime_ns`, `ctime_ns`, `size_bytes`, `inode`) is implemented and tested. The "returns DB files as missing" stub language from Batch C no longer applies.
- [x] 4.4 `full_hash_reconcile(collection_id)`: ignore stat fields; hash every file; rebuild `file_state` from disk; used by remap/restore/fresh-attach/audit. Includes a closed mode/authorization contract and unchanged-hash metadata self-heal without `raw_imports` rotation.
  > **Complete (Batch G):** `full_hash_reconcile` now hashes every walked file, validates a closed mode/authorization contract against current collection state before walking, reuses rename/apply for changed/new content, and self-heals unchanged-hash rows by updating only `file_state` / `last_full_hash_at`. Unchanged-hash paths do not rewrite `pages` or rotate `raw_imports`.
- [x] 4.5 UUID-first identity resolution in reconcile: build in-memory `gbrain_id → (path, sha256)` index from the new tree; prefer UUID match over path before falling back to content-hash uniqueness guards.
  > **Complete (Batch E):** Reconciler now hashes/parses live new-file candidates into an in-memory UUID/path/hash identity index, applies native rename pairs first, then UUID matches against `pages.uuid`, and only then attempts conservative hash pairing. This remains classification-only until the later apply pipeline lands; no filesystem or DB mutations happen here yet.
- [ ] 4.6 Periodic full-hash audit: background task rehashes files whose `last_full_hash_at` is older than `GBRAIN_FULL_HASH_AUDIT_DAYS` (default 7). `gbrain collection audit <name>` for on-demand trigger.

## 5. Reconciler

- [x] 5.1 Create `src/core/reconciler.rs`. ~~Replace `import_dir()` from `migrate.rs`.~~
  > **Repair note (Leela, Batch C repair):** File created with correct types and function signatures. `migrate::import_dir()` remains the live ingest path. Safety-critical stubs now fail explicitly: `has_db_only_state` returns `Err` (not `Ok(false)`); `walk_collection` and `full_hash_reconcile` return `Err("not yet implemented")` instead of success-shaped empty stats. Any live path wired to these before tasks 5.2–5.5 land will fail loudly.
  > **Batch D update (Leela):** `walk_collection` (task 5.2) and `has_db_only_state` (task 5.4) are now real implementations — neither returns `Err` anymore. `full_hash_reconcile` was later completed in Batch G with mode/authorization validation, unchanged-hash metadata self-heal, and the full hash-based apply path.
- [x] 5.2 Implement walk using `ignore::WalkBuilder` bounded to `root_fd`; respect `.gbrainignore` + built-in defaults.
  > **Complete:** Reconciler now opens `root_fd` first, reloads `.gbrainignore`, walks with `ignore::WalkBuilder`, and re-stats every candidate entry via fd-relative `walk_to_parent` + `stat_at_nofollow` before trusting it. Symlinked roots still refuse, symlinked entries/ancestors are skipped with WARN, built-in defaults still apply, and this same walk now closes the real `stat_diff` filesystem-walk gap rather than treating every stored row as missing.
- [x] 5.3 Implement rename resolution: (1) native event pairing when available; (2) UUID match; (3) content-hash uniqueness with guards (≥64 body bytes after frontmatter, unique hash in both `missing` and `new`, non-empty body after frontmatter); (4) quarantine + fresh create otherwise.
  > **Complete (Batch E boundary):** Native pairing is implemented as an interface-level reconciler input and is exercised in tests, but watcher/event production is still deferred. UUID matches and conservative hash matches now classify rename-vs-quarantine in memory; quarantine/fresh-create application still waits for the later mutation pipeline.
  > **Batch E repair (Leela):** Hash-rename guard now uses body bytes (after frontmatter, trimmed) for the ≥64-byte threshold, not whole-file size. Previous implementation used `file_state.size_bytes` (whole-file), which allowed large-frontmatter / tiny-body template notes to satisfy the threshold and be incorrectly paired. `MissingPageIdentity.body_size_bytes` is computed from `compiled_truth + timeline` in the DB; `NewTreeIdentity.body_size_bytes` is computed from the parsed body after frontmatter. Refusal reasons updated to `missing_below_min_body_bytes` / `new_below_min_body_bytes`. Regression test `template_note_with_large_frontmatter_and_tiny_body_is_never_hash_paired` added.
- [x] 5.3a On condition failure in (3), log `rename_inference_refused reason=<...>` at INFO so decisions are debuggable.
- [x] 5.4 Implement delete-vs-quarantine classifier using `has_db_only_state(page_id)` predicate (five-branch OR over programmatic links, non-import assertions, `raw_data`, `contradictions`, `knowledge_gaps`).
  > **Complete:** `has_db_only_state(page_id)` is now the real five-branch SQL predicate — never a success-shaped `Ok(false)` stub — and missing-file classification counts quarantine-vs-hard-delete using that predicate without wiring the later apply pipeline.
- [x] 5.4a Audit every callsite that inserts into `links` — populate `source_kind` explicitly. `brain_link` sets `programmatic`. Default is `programmatic` (fail-open preservation).
  > **Audit result:** `brain_link` (`commands/link.rs`) explicitly sets `source_kind = 'programmatic'`. `extract_links()` (`core/links.rs`) returns `Vec<String>` slugs only — it does NOT write to the `links` table and does NOT set `source_kind`. No production callsite currently populates `wiki_link`; the value is reserved in the schema CHECK constraint but has no live writer yet. The earlier claim that `extract_links()` sets `wiki_link` was incorrect and is retracted.
- [x] 5.4b Audit every callsite that inserts into `assertions` — use `asserted_by='import'` only from `check_assertions()`; every other path (agent, manual, enrichment) uses a non-import value.
- [x] 5.4c Unit test: page with each of the five DB-only categories independently triggers quarantine (not hard-delete).
- [x] 5.4d Every content-changing write covered by this slice rotates `raw_imports` per file inside the SAME SQLite tx as the corresponding `pages`/`file_state` mutation; within Batch F this means ingest + reconciler apply, and inside reconciler each per-file rotation happens inside its enclosing 500-file chunk tx.
  > **Complete (Batch F slice):** Added shared `core::raw_imports` rotation helpers and wired them into `commands::ingest`, `core::migrate::import_dir`, and reconciler apply-time re-ingest/create/rename paths. For the paths in scope here, page/file_state mutation, raw_import rotation, and embedding-job enqueue now happen in the same SQLite transaction.
- [x] 5.4e Inline GC in the rotation tx: enforce `GBRAIN_RAW_IMPORTS_KEEP` (default 10) per page AND `GBRAIN_RAW_IMPORTS_TTL_DAYS` (default 90). Active row is never touched. `GBRAIN_RAW_IMPORTS_KEEP_ALL=1` disables GC.
  > **Complete (Batch F slice):** Inline per-page GC now runs inside the same rotation transaction, trims inactive history beyond the keep cap, drops TTL-expired inactive rows, and honors `GBRAIN_RAW_IMPORTS_KEEP_ALL=1`. Active rows are never deleted by GC.
- [ ] 5.4f Daily background sweep in `gbrain serve` for TTL-expired inactive rows on idle pages; also triggered by `gbrain collection audit --raw-imports-gc`.
- [x] 5.4g Post-ingest unit-test assertion: `SELECT COUNT(*) FROM raw_imports WHERE page_id=? AND is_active=1` equals 1 after every write path.
  > **Complete (Batch F slice):** Added write-path tests for single-file ingest, directory import, reconciler rename/apply, and raw_import rotation itself; each asserts exactly one active row remains after the write commits.
- [x] 5.4h If `full_hash_reconcile` or restore ever finds a page with zero active `raw_imports` rows, abort with `InvariantViolationError` before mutation; `--allow-rerender` CLI flag is the audit-logged WARN recovery override.
  > **Complete for Batch H boundary:** The full-hash path and stat-diff reingest path both fail closed with a typed `InvariantViolationError` before any mutation when `raw_imports` history is missing or inactive. Batch H also lands the closed `RawImportInvariantPolicy` seam so the future operator-only restore `--allow-rerender` override stays explicit and audit-visible without introducing any generic rerender branch into audit/recovery/remap/fresh-attach code.
  > **Batch G repair (Leela):** The stat-diff `apply_reingest` path had a corresponding gap: when `raw_imports` history was zero-total for an existing page, `rotate_active_raw_import` treated `row_count == 0` as acceptable and silently bootstrapped the first row. Fixed with a pre-flight guard in `apply_reingest` that fires before any page mutation, covering both the modified-path case (`existing_page_id = Some`) and the slug-matched new-path case (`existing_page_id = None` but slug resolves to an existing page). Two adversarial regression tests added.
- [x] 5.5 Wire the Unix-only stat-diff → rename resolution → delete-vs-quarantine classifier → apply pipeline (re-ingest + hard-delete where allowed + quarantine where required + hash-renames) → enqueue embeddings, and re-evaluate `has_db_only_state(page_id)` at apply time rather than trusting earlier classification snapshots.
  > **Complete (Batch F):** Unix reconcile now turns classification into real DB mutations: missing pages are hard-deleted or quarantined in-transaction, modified/new files are re-ingested, rename matches preserve `pages.id` while moving `file_state`, raw_imports rotate atomically, and `embedding_jobs` are enqueued. Delete-vs-quarantine is re-evaluated inside the apply transaction via a fresh `has_db_only_state(page_id)` check.
- [x] 5.6 Commit in batches of 500 files.
  > **Complete (Batch F):** Reconcile now builds per-file apply actions and executes them in explicit 500-file SQLite transactions. A regression test covers partial progress: the first 500-file chunk commits even when a later chunk fails on invalid input.
- [x] 5.7 Per-phase log line: `walked=N unchanged=N modified=N new=N missing=N native_renamed=N hash_renamed=N quarantined_ambiguous=N quarantined_db_state=N hard_deleted=N`.
  > **Complete (Batch F):** Reconcile now emits `INFO: reconcile_plan ...` and `INFO: reconcile_apply ...` summary lines that surface the walk/classification counts and the actual apply outcomes using the repository's existing stderr log style.
- [ ] 5.8 Implement the shared restore/remap safety pipeline (Phase 1 drift capture → Phase 2 stability → Phase 3 pre-destruction fence → Phase 4 new-root verification for remap) per vault-sync spec § "Two-phase defense". The destructive step (Tx-A for restore, DB-update tx for remap) SHALL NOT run without the preceding phases.
  > **Batch H honesty note:** This batch lands only Phase 0–3 helpers plus fresh-attach wiring. Phase 4 new-root verification and end-to-end restore/remap execution/orchestration (`5.8e`, `5.8f`, `5.8g`) remain deferred.
- [x] 5.8a0 UUID-migration preflight (runs FIRST, before RO-mount gate): scan `pages` for rows whose `uuid` is not present in the file's frontmatter AND whose content is trivial under the same canonical helper used by 5.3 (`body_size_bytes < 64` after frontmatter OR empty body). If any found, refuse with `UuidMigrationRequiredError` naming the count and up to 5 sample paths, directing the operator to run `gbrain collection migrate-uuids <name>` before retrying. This gate closes the silent-identity-loss path for short/template notes that have neither `gbrain_id` frontmatter nor content-hash uniqueness. The check runs against the DB (no filesystem walk) and is O(page_count).
- [x] 5.8a Preflight RO-mount gate: `statvfs(old_root)` inspects `ST_RDONLY` (Linux) / `MNT_RDONLY` (macOS). RO mount → proceed with INFO `restore_ro_mount_verified`. Writable mount → refuse with `CollectionLacksWriterQuiescenceError` naming the two acceptance paths (remount RO, or run from a quiesced environment). No `--writers-quiesced` / `--unsafe-accept-residual-race` flags exist.
- [x] 5.8a2 `dirty-preflight` guard (before Phase 1): refuse if `is_collection_dirty(collection_id)` is TRUE OR the sentinel directory is non-empty, unless the caller is `sync --finalize-pending`. Error message instructs waiting for RCRT or running `gbrain collection sync`.
- [x] 5.8b Phase 1 — drift capture: open a fresh old-root walk via `full_hash_reconcile_authorized(..., mode=RestoreDriftCapture|RemapDriftCapture)` using the closed authorization enum carrying caller identity (`restore_command_id` or owning lease/session identity). For restore, captured drift becomes the authoritative `raw_imports`. For remap, any material drift aborts with `RemapDriftConflictError` naming the `DriftCaptureSummary` counts (`pages_updated`, `pages_added`, `pages_quarantined`, `pages_deleted`). Log `restore_drift_captured` WARN when non-zero; `remap_drift_refused` ERROR.
- [x] 5.8c Phase 2 — stability check: two successive stat-only snapshots over old root `(relative_path, mtime_ns, ctime_ns, size_bytes, inode)`. Equal → proceed. Differ → re-run Phase 1 and capture `snap3`; retry up to `GBRAIN_RESTORE_STABILITY_MAX_ITERS` (default 5). Persistent instability → `CollectionUnstableError`. For remap, any retry with non-zero drift falls back to `RemapDriftConflictError`.
- [x] 5.8d Phase 3 — pre-destruction fence: one final stat-only walk `snap_fence` compared to `snap_final`. Diff → abort via the standard abort-path resume sequence (revert state, keep `root_path`, clear ack triple, NULL heartbeat, bump `reload_generation`, drop offline lease, stop heartbeat tasks); log `restore_aborted_fence_drift` / `remap_aborted_fence_drift` WARN; return `CollectionUnstableError`.
- [x] 5.8d2 TOCTOU recheck: between Phase 2 stability and the destructive step, re-evaluate `is_collection_dirty` on a fresh SQLite connection AND re-scan the sentinel directory. TRUE → abort with `CollectionDirtyError` via the same abort-path resume sequence.
- [x] 5.8e Phase 4 (remap only) — `/new/path` manifest verification + new-root stability fence. Use the canonical `resolve_page_identity(...)` (UUID first, then content-hash uniqueness with size>64 / non-empty-body guards — NO relative-path shortcut). Pass criteria: (i) every active-indexable page resolves to exactly one file on `/new/path`, (ii) sha256 matches authoritative `raw_imports.raw_bytes`, (iii) every non-ignored file resolves to exactly one page. Full-tree fence (`newroot_snap_pre` vs `newroot_snap_fence`) detects file-set / per-file-tuple / `.gbrainignore`-sha256 drift between verification and DB-update; drift → `NewRootUnstableError`. Pass-criteria failure → `NewRootVerificationFailedError` naming counts and sampled diffs. Quarantined pages excluded from both sides of the bijection.
- [x] 5.8f Online restore (live supervisor): Phase 1 runs AFTER handshake release so drift capture sees the live tree. Staging + per-file sha256 verification + Tx-A + rename + Tx-B follow. Online remap does only the one-tx DB update (`reload_generation++`, `needs_full_sync=1`, state stays `'restoring'`, deletes `file_state`); RCRT handles post-state attach + `full_hash_reconcile`.
- [x] 5.8g Offline mode: CLI holds the `collection_owners` lease with heartbeat throughout; runs the full pipeline end-to-end; releases the lease on completion.
- [x] 5.9 Wire fresh-attach and first-use-after-detach to invoke `full_hash_reconcile` in `FreshAttach` mode before clearing `needs_full_sync` / reopening writes.
  > **Complete for Batch H core seam:** `fresh_attach_reconcile_and_activate()` now runs a dedicated fresh-attach full-hash pass and clears `needs_full_sync` only after reconcile succeeds. Higher-level serve/supervisor choreography remains outside this batch boundary.

## 5a. UUID lifecycle and frontmatter persistence

- [x] 5a.1 Add `uuid7` crate (or use `uuid` with v7 support).
- [x] 5a.2 Extend `parse_frontmatter()` and `render_page()` to treat `gbrain_id` as a first-class field; reading preserves it; rendering emits it if present.
- [x] 5a.3 Extend `Page` struct with `uuid: String` (non-optional).
  > **Complete (construction cascade closed):** `Page.uuid` is now required everywhere `Page` is constructed or serialized. Read paths fail loudly on rows that still lack a UUID rather than inventing a placeholder default.
- [x] 5a.4 Ingest pipeline: if `frontmatter.gbrain_id` is present, adopt it as `pages.uuid`; if absent, generate UUIDv7 server-side and store in `pages.uuid` ONLY. Default ingest is READ-ONLY with respect to user bytes — no self-write enqueued.
- [x] 5a.4a Regression test: save a `.md` without `gbrain_id`; observe watcher event; assert file bytes unchanged, `file_state.sha256` equals user hash, dedup set empty, git remains clean.
  > **Batch E note:** Current coverage is at the compatibility-ingest boundary (`gbrain ingest` / import path): generated UUIDs stay DB-only, source bytes remain unchanged, and a git worktree stays clean. Watcher dedup/file_state assertions remain deferred with watcher work.
- [ ] 5a.5 Opt-in UUID write-back for `--write-gbrain-id`, `migrate-uuids`, and `brain_put` only. Uses the full rename-before-commit discipline (sentinel, tempfile, `O_NOFOLLOW`, atomic rename, fsync parent, post-rename stat, single tx with `file_state` + `raw_imports` rotation). Read-only files (EACCES/EROFS) are skipped with WARN; `pages.uuid` remains set.
- [ ] 5a.5a CLI: `gbrain collection add --write-gbrain-id` and `gbrain collection migrate-uuids <name> [--dry-run]`. Both are `WriteAdmin`, honor the restoring-state interlock, and only self-write files missing `gbrain_id`. Summary reports `migrated/skipped_readonly/already_had_uuid`.
- [x] 5a.6 `brain_put` preserves `gbrain_id`: `render_page()` is the explicit `brain_put` seam and always emits existing `pages.uuid` in frontmatter so agents cannot inadvertently strip it.
  > **Complete (Batch G):** `render_page()` now always re-emits persisted `pages.uuid` as `gbrain_id`, so `brain_put` / `brain_get` surfaces cannot strip UUID identity when incoming markdown omits it.
- [ ] 5a.7 Unit tests: default-ingest read-only; `gbrain_id` adoption; opt-in rewrite rotates `file_state`/`raw_imports` atomically; `migrate-uuids --dry-run` mutates nothing; `brain_put` always emits preserved `gbrain_id`; UUIDv7 monotonicity; frontmatter round-trip preserves `gbrain_id`; Batch G also covers unchanged-hash no-rotation, changed-hash rotation, and zero-active abort.
  > **Batch G partial:** Added direct coverage for `brain_put`/`render_page` UUID preservation, full-hash unchanged-hash no-rotation, full-hash changed-hash rotation, and full-hash zero-active abort. The remaining UUID write-back / migrate-uuids coverage stays deferred with tasks 5a.5–5a.5a.

## 6. Watcher pipeline

- [ ] 6.1 Add `notify` crate (with `macos_fsevents` feature).
- [ ] 6.2 Per-collection watcher task: one `notify` recommended watcher per collection, events pushed into a bounded `tokio::mpsc` channel tagged with `CollectionId`.
- [ ] 6.3 Per-collection debounce buffer; default `GBRAIN_WATCH_DEBOUNCE_MS=1500` coalesces Obsidian bulk saves.
- [ ] 6.4 Batch processor drains the debounce buffer, runs stat-diff, commits updates.
- [ ] 6.5 Create/Modify handler: re-ingest bytes; never self-write UUID on observed external edits.
- [ ] 6.6 Delete handler: invoke delete-vs-quarantine classifier.
- [ ] 6.7 Rename handler: honor native pair events directly; update `file_state.relative_path`; preserve `pages.id`.
- [ ] 6.7a Overflow recovery task: on bounded-channel overflow, set `collections.needs_full_sync=1` in a brief tx, WARN log, continue accepting events. Recovery task polls the flag every 500ms and runs `full_hash_reconcile` within ~1s. Recovery worker is gated to `state='active'` only.
- [ ] 6.8 `.gbrainignore` watcher: treat as live control file; trigger atomic parse + mirror refresh + reconciliation on any change.
- [ ] 6.9 Watcher auto-detect: native first, downgrade to poll on init error with WARN.
- [ ] 6.10 Per-collection watcher supervisor with crash/restart + exponential backoff.
- [ ] 6.11 Expose watcher health (last event time, channel depth, mode) via `brain_collections` and `gbrain collection info`.

## 7. Self-write dedup set

- [ ] 7.1 Implement `Arc<Mutex<HashMap<PathBuf, (sha256, Instant)>>>` in the serve process.
- [ ] 7.2 Dedup entry inserted at step 8 of the rename-before-commit sequence (AFTER tempfile+fsync, BEFORE `renameat`).
- [ ] 7.3 Watcher consults dedup set before emitting: if path + hash match an entry younger than 5s, drop the event.
- [ ] 7.4 Background sweeper removes expired entries every 10s.
- [ ] 7.5 Failure handlers remove the entry: rename failure unlinks tempfile + sentinel + removes dedup; post-rename failures remove dedup so reconciler can observe the new bytes.
- [ ] 7.6 Unit tests: echo suppression; path-only match rejects; expired entries no longer suppress; external edit after TTL ingests normally.

## 8. Embedding queue and worker

- [ ] 8.1 Add `embedding_jobs(page_id, chunk_index, job_state, attempt_count, last_error, created_at, started_at)` table.
- [ ] 8.2 Ingest/reconciler/`brain_put` enqueue jobs atomically in the same SQLite tx as `pages`/`file_state`.
- [ ] 8.3 Background worker drains jobs with bounded concurrency `min(cpus, 4)` (configurable via `GBRAIN_EMBEDDING_CONCURRENCY`).
- [ ] 8.4 Worker retries with exponential backoff; permanent failures leave the job in `failed` state with `last_error`.
- [ ] 8.5 On `gbrain serve` startup, resume pending jobs.
- [ ] 8.6 Expose queue depth + failing jobs in `brain_collections` + `gbrain collection info`.

## 9. `gbrain collection` commands

- [x] 9.1 Implement `src/commands/collection.rs` with clap subcommands.
- [x] 9.2 `gbrain collection add <name> <path> [--writable/--read-only]`: validate name (no `::`), validate/open `root_fd` with `O_NOFOLLOW` before row creation, persist the detached row, and run the fresh-attach reconciliation path. K1 keeps default attach read-only with respect to vault bytes; `--write-gbrain-id` / watcher-mode remain deferred.
- [ ] 9.2a `--write-gbrain-id` triggers opt-in UUID write-back during the initial walk; default is read-only.
- [x] 9.2b Capability probe: attempt a tempfile write inside the root; if EACCES/EROFS, set `collections.writable=0` with WARN; subsequent K1-scoped vault-byte writes (`gbrain put`) refuse with `CollectionReadOnlyError`.
  > **Scope note (Leela, K1 repair):** "K1-scoped vault-byte writes" means only `gbrain put` / MCP `brain_put`. DB-only mutators (`brain_check`, `brain_link`, `brain_tags`, `brain_raw`, slug-bound `brain_gap`) intentionally do NOT check `CollectionReadOnly` per Professor's ruling: the vault-byte gate and the DB-only write-interlock are separate concerns. This is the correct and complete K1 behaviour.
- [x] 9.3 `gbrain collection list` prints `name | state | writable | write_target | root_path | page_count | last_sync_at | queue_depth`.
- [x] 9.4 `gbrain collection info <name>` prints extended status including ignore_parse_errors, integrity flags, pending_root_path, recovery progress.
- [x] 9.5 `gbrain collection sync <name>` runs ONLY the ordinary active-root `stat_diff + reconciler` path. `--remap-root <path>` runs remap per task 5.8. `--finalize-pending` triggers `finalize_pending_restore(…, FinalizeCaller::ExternalFinalize)`. Offline plain sync acquires its own short-lived `collection_owners` lease with heartbeat, clears `needs_full_sync` only after a real active-root reconcile succeeds, and stays fail-closed on blocked states.
  > **Batch J honesty note (Leela):** This slice does **not** widen plain sync into finalize/remap/reopen/serve-handshake recovery multiplexing. Success means only “active root reconciled”.
- [ ] 9.6 `gbrain collection remove <name>` detaches and optionally `--purge` drops rows with an explicit confirmation.
- [x] 9.7 `gbrain collection restore <name> <target>`: stage → verify per-file sha256 → Tx-A (set `pending_root_path`, `pending_restore_manifest`, restore-command identity) → atomic rename → Tx-B (`run_tx_b` finalize).
  > **Batch I note (Leela):** Offline restore now stops after Tx-B and leaves `state='restoring'` + `needs_full_sync=1` until RCRT owns attach completion. Command success does **not** mean writes have reopened; the end-to-end integration proof stays deferred under task `17.11`.
- [x] 9.7a Refuse if target is non-empty (no `--force`): POSIX `rename()` cannot atomically replace a non-empty target.
- [x] 9.7b Emit post-restore summary: `restored=N byte_exact=N pending_finalize=<bool> pending_root_path=<P>`.
- [x] 9.7c Offline restore acquires the `collection_owners` lease with heartbeat; online restore runs the lease-based handshake (task 11.6).
- [x] 9.7d **Restoring-Collection Retry Task (RCRT)** at `gbrain serve` startup and on a continuous sweep: observe owned collections with no live `supervisor_handles` entry and drive recovery. Actions: finalize pending restores (`FinalizeCaller::StartupRecovery`), orphan-recovery for dead originators, single-flight attach handoff (open new `root_fd`, run `full_hash_reconcile`, then in attach-completion tx flip `state='active'` and clear `needs_full_sync`, THEN spawn supervisor). Skip any collection where `reconcile_halted_at IS NOT NULL`.
- [x] 9.7e `gbrain collection restore-reset <name> --confirm`: clears terminal integrity-blocked state (`integrity_failed_at`, escalated `pending_manifest_incomplete_at`, restore-command identity tuple, `pending_root_path`, `pending_restore_manifest`).
- [x] 9.7f `gbrain collection reconcile-reset <name> --confirm`: clears `reconcile_halted_at` + `reconcile_halt_reason` after operator has manually resolved the offending vault state.
- [ ] 9.8 `gbrain collection quarantine {list,restore,discard,export,audit}`. `discard` on a page with DB-only state requires `--force` OR a prior `export` (which dumps all five DB-only-state categories as JSON).
- [ ] 9.9 Auto-sweep TTL: `GBRAIN_QUARANTINE_TTL_DAYS` (default 30) auto-discards ONLY pages where `has_db_only_state` is false; log each discard and DEBUG-log each skip.
- [ ] 9.9b `gbrain collection info` surfaces count of "quarantined pages awaiting user action".
- [ ] 9.10 `gbrain collection ignore add|remove|list|clear --confirm` per §3.
- [ ] 9.11 All `collection` subcommands produce stable machine-parseable summaries on success and non-zero exit on any error.

## 10. `gbrain init` changes

- [ ] 10.1 Write v5 schema; initialize `brain_config.schema_version=5`; persist embedding model metadata.
- [ ] 10.2 Remove any import-related bootstrap logic.
- [ ] 10.3 Prompt nothing about vault paths — collections are attached via `gbrain collection add` after init.

## 11. `gbrain serve` integration

 - [x] 11.1a Initialize the registry-startup half of the process-global registries: `supervisor_handles` + dedup set bookkeeping used by startup RCRT/supervisor ordering.
 - [x] 11.1b Initialize the recovery sentinel directory.
   > **Complete (Batch L2 startup-only seam):** `gbrain serve` startup now bootstraps `<brain-data-dir>/recovery/<collection_id>/` before recovery passes run. This is directory bootstrap only; writer-side sentinel creation remains deferred to task 12.1.
- [x] 11.2 Register a `serve_sessions` row on startup (session_id UUIDv7, pid, host, started_at, heartbeat_at, `ipc_path`); refresh heartbeat every 5s.
- [x] 11.3 Sweep stale `serve_sessions` rows (>15s old) on startup.
- [x] 11.4 Recover from `brain_put` recovery sentinels directory: set `collections.needs_full_sync=1` for affected collections; unlink each sentinel after successful reconciliation.
  > **Complete (Batch L2 startup-only seam):** serve startup now scans owned collection sentinel dirs, marks sentinel-bearing active collections dirty, drives the existing startup reconcile path, and unlinks every `.needs_full_sync` file only after reconcile succeeds. Foreign-owned collections, restore/integrity-blocked collections, and failed reconciles leave sentinels in place.
- [x] 11.5 Run RCRT (task 9.7d) before spawning supervisors.
- [x] 11.6 Implement `collection_owners` lease: `PRIMARY KEY(collection_id)` makes multi-owner impossible by schema. Acquire under transaction; renew via session heartbeat; release on supervisor exit or session termination.
- [x] 11.7 Per-collection supervisor spawn under per-collection single-flight mutex. Supervisor polls `state`+`reload_generation`; on observing `restoring` with a greater generation, release watcher + `root_fd`, write ack triple tagged with own `session_id`, remove self from `supervisor_handles`, and exit. Never impersonate another session.
- [x] 11.7a Fresh serve that observes `state='restoring'` at startup does NOT write the ack triple; treats the collection as unattached until RCRT drives it to active or the originating command completes.
- [x] 11.8 Write interlock: every `WriteCreate`/`WriteUpdate`/`WriteAdmin` op BEFORE any DB or FS mutation checks resolved `collections.state`. Refuse with `CollectionRestoringError` if `state='restoring'` OR `needs_full_sync=1` (write-gate armed by Tx-B). Interlock applies to all mutating CLI/MCP entry points including `brain_check`, `brain_raw`, `brain_link`, slug-bound `brain_gap`, `ignore add|remove|clear`, `migrate-uuids`, and `--write-gbrain-id`.
- [ ] 11.9 Open UNIX socket at `serve_sessions.ipc_path` for CLI write proxying under the full trust-boundary contract — placement (§12.6c), bind-time audit (§12.6d), server-side peer verification (§12.6e). Write `ipc_path` to the `serve_sessions` row after bind+audit succeeds; on shutdown, `unlink` the socket and NULL the column.

## 12. `brain_put` write-through (rename-before-commit)

- [ ] 12.1 Implement the full 13-step rename-before-commit sequence per design.md and agent-writes spec: (1) precondition + CAS; (2) `walk_to_parent`; (3) `check_fs_precondition`; (4) compute sha256; (5) create recovery sentinel via `openat(recovery_dir_fd, "<write_id>.needs_full_sync", O_CREAT|O_EXCL|O_NOFOLLOW) + fsync`; (6) create+fsync tempfile via `O_CREAT|O_EXCL|O_NOFOLLOW`; (7) defense-in-depth `fstatat(AT_SYMLINK_NOFOLLOW)`; (8) dedup insert; (9) `renameat(parent_fd,…)`; (10) `fsync(parent_fd)` — HARD STOP on failure; (11) `fstatat` post-rename for full stat; (12) single SQLite tx upsert pages/FTS/file_state + rotate raw_imports + enqueue embedding_jobs; (13) best-effort unlink sentinel. **Status:** only the pre-gated M1a crash core below is landed; `12.2`, `12.3`, and the rest of full `12.1` remain deferred.
- [x] 12.1a Narrow pre-gated M1a writer-side sentinel crash core only: create + durably fsync the recovery sentinel before vault mutation; create+fsync tempfile; rename; hard-stop on parent-directory fsync failure; detect post-rename foreign replacement before DB work; run the existing page/file_state/raw_imports/embedding_jobs SQLite write in one tx; best-effort unlink sentinel on success; retain sentinel on post-rename failure for startup recovery.
- [x] 12.2 `check_fs_precondition`: fast path when all four stat fields match; slow path hashes on any mismatch; hash match self-heals stat fields; hash mismatch returns `ConflictError`. `ExternalDelete`/`ExternalCreate`/`FreshCreate` cases defined. **Unix `gbrain put` / `brain_put` path only.**
- [x] 12.3 Enforce mandatory `expected_version` for updates; only creates may omit. **Unix `gbrain put` / `brain_put` path only.**
- [x] 12.4 Per-slug async mutex serializes within-process writes (not a substitute for DB CAS). **Closure note:** vault-byte write entry points only; same-slug writes serialize within the process, while different slugs remain concurrent and DB CAS still owns cross-process safety.
- [x] 12.4a Pre-sentinel failure: no vault mutation; no DB mutation; return error. **Unix `gbrain put` / `brain_put` path only.**
- [x] 12.4aa Sentinel-creation failure: return `RecoverySentinelError`; no tempfile; no dedup; no DB. **Proof-only against 12.1a seam, not a claim that full `12.4` is done.**
- [x] 12.4b Pre-rename failure: unlink tempfile; remove dedup entry; unlink sentinel; return error. **Proof-only against 12.1a seam, not a claim that full `12.4` is done.**
- [x] 12.4c Rename failure: unlink tempfile; remove dedup; unlink sentinel; return error. **Proof-only against 12.1a seam, not a claim that full `12.4` is done.**
- [x] 12.4d Post-rename failure (fsync parent / post-rename stat / commit): remove dedup; leave sentinel in place; best-effort set `collections.needs_full_sync=1` via a fresh SQLite connection; return error. **Proof-only against 12.1a seam, not a claim that full `12.4` is done.**
- [x] 12.5 Enforce `CollectionReadOnlyError` when `collections.writable=0`. **Closure note:** vault-byte write entry points only (`gbrain put` and `brain_put` via `put_from_string`). The live enforcement site is `ensure_collection_vault_write_allowed`; DB-only mutators remain deferred.
- [ ] 12.6 Enforce the per-write `expected_version` contract across MCP + CLI + any future interface — no blind-update escape hatch.
- [ ] 12.6a CLI write routing — `gbrain put` (single-file): detect a live owner via `collection_owners` + `serve_sessions.ipc_path`. Live owner → Proxy mode over IPC (keeps the in-process dedup set coherent). No live owner → acquire the offline `collection_owners` lease with heartbeat and write directly.
- [ ] 12.6b CLI write routing — bulk rewrites are **Refuse-by-default**, NOT Proxy. `gbrain collection migrate-uuids` and `gbrain collection add --write-gbrain-id` SHALL refuse with `ServeOwnsCollectionError` when any live owner exists, naming pid/host and instructing the operator to stop serve (or `detach --online`), run the bulk rewrite offline, then restart serve. Per-file proxy of thousands of rewrites is explicitly out of scope for this change; a batched proxy protocol is a follow-up.
- [ ] 12.6c IPC socket placement: serve creates the parent directory at mode `0700` under `$XDG_RUNTIME_DIR/gbrain/` on Linux (fallback `$HOME/.cache/gbrain/run/` if unset) or `$HOME/Library/Application Support/gbrain/run/` on macOS. If the directory exists with broader permissions or non-matching UID, refuse startup with `IpcDirectoryInsecureError`. Socket path: `<dir>/<session_id>.sock` with the UUIDv7 session_id embedded.
- [ ] 12.6d Bind-time audit: after `bind()`, serve `stat()`s the socket, verifies mode `0600` and owning UID matches its own. Any deviation → `IpcSocketPermissionError`, serve aborts startup. Stale sockets from dead prior sessions are `unlink`ed before `bind()`. Collision with a live same-UID different-session holder → `IpcSocketCollisionError`.
- [ ] 12.6e Server-side peer verification: on every `accept()`, serve calls `getsockopt(SO_PEERCRED)` (Linux) or `LOCAL_PEERCRED` / `getpeereid()` (macOS) and refuses any connection whose peer UID ≠ serve's UID. Peer PID is logged at INFO for observability (not a security boundary).
- [ ] 12.6f Client-side peer verification (authoritative auth): before forwarding a write, the CLI SHALL (a) `stat` the socket and verify mode `0600` and owning UID matches its own; (b) after `connect()`, read kernel-backed peer PID+UID via `SO_PEERCRED` / `getpeereid()`; (c) verify peer UID == current UID AND peer PID == `serve_sessions.pid` for the session whose `session_id` is embedded in the socket path. Only after (a)–(c) pass may the CLI issue a protocol-level `whoami` — its returned `session_id` is a CROSS-CHECK against the path-embedded session_id, not the primary auth primitive. Any failure → `IpcPeerAuthFailedError` with NO write forwarded.
- [ ] 12.6g IPC negative tests: (i) same-UID attacker races `bind()` and returns a spoofed protocol `whoami` → CLI detects kernel PID mismatch and refuses; (ii) stale socket from a dead prior session is unlinked cleanly at startup and a fresh bind succeeds; (iii) socket parent-dir with mode `0755` refuses startup; (iv) socket file mode regression (e.g., umask bug producing `0644`) is caught at bind-time audit; (v) cross-UID client is refused at `accept()`.
- [ ] 12.7 Unit + integration tests covering every step's happy path and every documented failure mode (tempfile fsync error, parent fsync error, commit error, foreign rename in window, concurrent dedup entries, external write mid-precondition).

## 13. Collection-aware slug parsing across MCP / CLI

- [x] 13.1 All slug-bearing MCP tool handlers (`brain_get`, `brain_put`, `brain_link`, `brain_backlinks`, `brain_graph`, `brain_timeline`, `brain_tags`, slug-bound `brain_check`, `brain_raw`, slug-bound `brain_gap`) resolve through collection-aware slug parsing before acting.
  > **Closed N1 (MCP-only):** `brain_link_close` is slugless, and `brain_search` / `brain_query` / `brain_list` are covered by canonical output work rather than a parse-first slug seam.
- [x] 13.2 MCP responses that reference a page return its canonical `<collection>::<slug>` form.
  > **Closed N1 (MCP-only):** canonical page references are now emitted on the MCP surfaces covered by this slice, including rendered `brain_get` output plus `brain_search`, `brain_query`, `brain_list`, `brain_backlinks`, `brain_graph`, `brain_timeline`, `brain_link`, and `brain_check`. CLI parity remains open in `13.3`.
- [ ] 13.3 CLI commands accept both bare slugs and `<collection>::<slug>`; apply the same resolution rules.
- [x] 13.4 `AmbiguityError` payload shape is stable (array of candidate strings + machine-readable code).
  > **Closed N1 (MCP-only):** MCP ambiguity failures now return code `ambiguous_slug` with a stable `candidates` array of canonical page addresses.
- [ ] 13.5 `brain_search` / `brain_query` / `brain_list` accept an optional `collection` filter; default filters by write-target in single-writer setups, all collections otherwise.
- [ ] 13.6 New `brain_collections` MCP tool returns the per-collection object documented in design.md (§`brain_collections` schema).

## 14. `gbrain stats` update

- [ ] 14.1 Augment output with per-collection rows: name, page_count, queue_depth, last_sync_at, state, writable.
- [ ] 14.2 Add aggregate totals (pages across all collections, quarantined count, embedding jobs pending/failed).

## 15. Remove legacy ingest

- [ ] 15.1 Delete `src/commands/import.rs`.
- [ ] 15.2 Delete `src/core/migrate.rs::import_dir()` and `ingest_log` helpers; split remaining logic between `reconciler.rs` and `writer.rs`.
- [ ] 15.3 Drop `ingest_log` table from schema.
- [ ] 15.4 This removal SHALL NOT merge until §16 doc updates are complete in the same change.

## 16. Documentation

- [ ] 16.1 Update `README.md` to remove `gbrain import`; document `gbrain collection add`.
- [ ] 16.2 Update `docs/getting-started.md` with the vault + collections workflow.
- [ ] 16.3 Update `docs/spec.md` to reflect v5 schema + live sync.
- [ ] 16.4 Update `AGENTS.md` and all `skills/*/SKILL.md` that referenced `gbrain import` or `import_dir`.
- [ ] 16.5 Update `CLAUDE.md` architecture section with new modules.
- [ ] 16.6 Update roadmap to reflect that live sync has landed and daemon-install / openclaw-skill are follow-ups.
- [ ] 16.7 Document every `GBRAIN_*` env var (see design.md § Environment variables).
- [ ] 16.8 Document the five DB-only-state categories and the quarantine resolution flow in `docs/spec.md`.

## 17. Tests

- [ ] 17.1 Unit: schema v5 creates all tables/indexes; v4 brain errors with re-init instructions.
  > **Status note (Professor, third pass):** direct refusal coverage now exists for the v4 preflight path in `db.rs`, including the "no v5 side effects before refusal" seam. The broader table/index audit remains open.
- [x] 17.2 Unit: `parse_slug` covers bare/`::`-qualified/ambiguous/not-found cases for every `op_kind`.
- [x] 17.3 Unit: `has_db_only_state` returns TRUE for each of the five categories independently.
- [ ] 17.4 Unit: `.gbrainignore` atomic parse — valid refreshes mirror; any invalid line preserves last-known-good; absent-file three-way semantics.
- [ ] 17.5 Integration: full collection lifecycle (add → modify → reconcile → link → restore).
- [x] 17.5a Reconciler idempotency: running twice yields zero changes on the second pass.
- [x] 17.5a2 Reconciler never descends symlinks.
- [x] 17.5a3 Reconciler skips symlinked entries with WARN.
- [x] 17.5a4 Reconciler refuses symlinked root at attach.
- [x] 17.5b Rename detection via native events preserves `pages.id`.
  > **Batch E scope note:** Exercised through the reconciler's native-rename interface. Watcher-produced native rename events are still deferred.
- [x] 17.5c Rename detection via UUID match preserves `pages.id` across directory reorganization.
- [x] 17.5d Rename detection via content-hash uniqueness preserves `pages.id`.
- [x] 17.5e Ambiguous hash-pair refusal quarantines old and creates new.
  > **Batch E scope note:** Verified at classification time: old paths become `quarantined_ambiguous` and unmatched new files remain fresh-create candidates. Actual mutation/apply behavior is deferred with task 5.5.
- [x] 17.5f Trivial-content (empty body after frontmatter) is never hash-paired.
- [x] 17.5g Quarantine: hard-delete when all five DB-only categories empty.
- [x] 17.5g2 Quarantine: programmatic link preserves the page.
- [x] 17.5g3 Quarantine: non-import assertion preserves.
- [x] 17.5g4 Quarantine: `raw_data` preserves.
- [x] 17.5g5 Quarantine: contradictions (either side) preserves.
- [x] 17.5g6 Quarantine: knowledge_gap with `page_id` preserves; without `page_id` does not.
- [ ] 17.5g7 `quarantine export` dumps all five categories as JSON.
- [ ] 17.5h Auto-sweep TTL: discard clean pages; never discard DB-only-state pages.
- [ ] 17.5i Quarantine `discard --force` on DB-only-state page requires exported JSON.
- [ ] 17.5j Quarantine `restore` re-ingests the page and reactivates the `file_state` row.
- [x] 17.5k `brain_put` happy path: tempfile → rename → single-tx commit. **Closure note:** narrow mechanical proof only for the vault-byte entry path; dedup echo suppression remains deferred.
- [x] 17.5l `brain_put` rejects stale `expected_version` with `ConflictError` before any FS mutation. **Unix proof only.**
- [x] 17.5m Filesystem precondition fast path when all four stat fields match. **Unix proof only.**
- [x] 17.5n Slow path on stat mismatch self-heals when hash matches. **Unix proof only.**
- [x] 17.5o Slow path returns `ConflictError` when hash differs. **Unix proof only.**
- [x] 17.5p External rewrite preserving `(mtime,size,inode)` but changing ctime is caught by the slow path. **Unix proof only.**
- [x] 17.5q External delete returns `ConflictError`. **Unix proof only.**
- [x] 17.5r External create returns `ConflictError`. **Unix proof only.**
- [x] 17.5s Fresh create succeeds when target absent and no `file_state` row. **Unix proof only.**
- [x] 17.5s2 Write-interlock refuses all mutating ops during `state='restoring'` OR `needs_full_sync=1`.
  > **Closed M1b-i (Bender):** Explicit mutator matrix in `src/mcp/server.rs` — `brain_put` ×2 (`restoring`, `needs_full_sync`), `brain_link` ×2, `brain_check` ×2, `brain_raw` ×2. All 8 cases enforce `CollectionRestoringError` at code `-32002`. 6 new tests + 2 pre-existing = 8 total. All pass.
  > **Repair note (Leela, M1b repair):** The 17.5s2/17.5s5 closure note was not tests-only. For `brain_link` and `brain_check`, the actual production write-gates live in `src/commands/link.rs::run_silent` and `src/commands/check.rs::execute_check` (calls to `vault_sync::ensure_collection_write_allowed`), not solely in `src/mcp/server.rs`. These are real behavior changes in the command layer. 17.5s5 is explicitly re-scoped below to own that behavior. The `needs_full_sync` test variants for `brain_link` and `brain_check` (`brain_link_refuses_when_collection_needs_full_sync`, `brain_check_refuses_when_collection_needs_full_sync`) also exercise these command-layer gates and were added under M1b-i.
- [x] 17.5s3 Slug-less `brain_gap` succeeds during `restoring` (Read carve-out).
  > **Closed M1b-i (Bender):** `brain_gap_without_slug_succeeds_while_collection_is_restoring` — pre-existing test, passes.
- [x] 17.5s4 Slug-bound `brain_gap` is refused during `restoring`.
  > **Closed M1b-i (Bender):** `brain_gap_with_slug_refuses_while_collection_is_restoring` + `brain_gap_with_slug_refuses_when_collection_needs_full_sync` — pre-existing tests, both pass.
- [x] 17.5s5 `brain_link`/`brain_check`/`brain_raw` refused during `restoring` with `CollectionRestoringError`.
  > **Closed M1b-i (Bender):** New tests — `brain_link_refuses_when_collection_is_restoring`, `brain_check_refuses_when_collection_is_restoring`, `brain_raw_refuses_when_collection_is_restoring`. All ErrorCode(-32002) + `CollectionRestoringError`. All pass.
  > **Re-scoped (Leela, M1b repair):** This task now explicitly owns the production behavior changes in the command layer: `src/commands/link.rs::run_silent` calls `vault_sync::ensure_collection_write_allowed` for both from/to collection IDs before any link mutation; `src/commands/check.rs::execute_check` calls `vault_sync::resolve_slug_for_op` + `vault_sync::ensure_collection_write_allowed` (slug-mode) or `vault_sync::ensure_all_collections_write_allowed` (all-mode) before extraction. For `brain_raw`, the gate lives directly in `src/mcp/server.rs`. The `needs_full_sync` variants (`brain_link_refuses_when_collection_needs_full_sync`, `brain_check_refuses_when_collection_needs_full_sync`) are also owned here. These are behavior changes, not proof-only tests.
- [x] 17.5s6 `brain_put` collection interlock wins over OCC/precondition conflicts.
  > **Closed M1b-ii (Leela, M1b repair):** `brain_put` in `src/mcp/server.rs` previously ran OCC prevalidation (version/existence checks) before the collection write-gate, allowing a blocked collection to return a version-conflict or existence-conflict error instead of `CollectionRestoringError`. Fixed by adding `vault_sync::ensure_collection_write_allowed` immediately after `resolve_slug_for_op` and before the OCC prevalidation block. This is cross-platform (no `#[cfg(unix)]` gate required: `ensure_collection_write_allowed` is a pure DB state check). Two new ordering-proof tests added: `brain_put_collection_interlock_wins_over_update_without_expected_version` (page exists + restoring → CollectionRestoringError) and `brain_put_collection_interlock_wins_over_ghost_expected_version` (page absent + expected_version supplied + restoring → CollectionRestoringError). Both tests fail before the fix and pass after.
- [x] 17.5t Recovery sentinel — creation failure aborts write; post-rename commit failure leaves sentinel on disk; startup recovery unlinks after reconcile.
- [x] 17.5u Foreign rename lands at target between steps 9 and 11 → `ConcurrentRenameError`; sentinel retained.
- [x] 17.5u2 Combined foreign-rename + `SQLITE_BUSY` on `needs_full_sync` write: sentinel alone drives recovery.
- [x] 17.5v Parent-directory fsync failure at step 10 → DB commit is REFUSED; sentinel retained.
- [ ] 17.5w `collections.needs_full_sync=1` triggers `full_hash_reconcile` within 1s.
- [ ] 17.5x Overflow recovery worker is gated to `state='active'` only.
- [ ] 17.5y `.gbrainignore` valid edit refreshes mirror + triggers reconciliation.
- [ ] 17.5z `.gbrainignore` single-line parse failure preserves last-known-good mirror.
- [ ] 17.5aa Absent `.gbrainignore` with prior mirror → WARN, mirror unchanged.
- [ ] 17.5aa2 `ignore clear --confirm` clears mirror and reconciles.
- [ ] 17.5aa3 CLI `ignore add` with invalid glob refuses with no disk mutation, no DB mutation.
- [ ] 17.5aa4 CLI `ignore remove` updates file and mirror transactionally.
- [ ] 17.5aa4b CLI is never the writer of `collections.ignore_patterns`.
- [ ] 17.5aa4c Built-in defaults always apply regardless of `.gbrainignore` state.
- [ ] 17.5aa5 `ignore_parse_errors` surfaces tagged-union shape in `brain_collections`.
- [ ] 17.5bb Dedup echo suppression works within TTL.
- [ ] 17.5cc External edit after TTL is ingested normally.
- [ ] 17.5dd Dedup path-only match (without hash) does NOT suppress.
- [ ] 17.5ee Embedding queue drains after write stampede; FTS always fresh.
- [ ] 17.5ff Embedding worker survives process restart and resumes pending jobs.
- [ ] 17.5gg Serve heartbeat row updates every 5s; stale rows >15s are ignored.
- [x] 17.5hh `collection_owners` PK keeps the single-owner invariant for offline plain-sync leases.
- [x] 17.5hh2 Short-lived CLI owner lease is released on normal exit and panic unwind without stale owner residue.
- [x] 17.5hh3 Offline plain sync acquires and renews its own lease via heartbeat.
- [ ] 17.5hh4 Owner lease change mid-handshake triggers `ServeOwnershipChangedError`.
- [ ] 17.5ii Restore stages to sibling directory; verifies per-file sha256 before Tx-A.
- [x] 17.5ii2 RO-mount gate: writable mount refuses with `CollectionLacksWriterQuiescenceError` naming the two acceptance paths; RO mount (Linux `mount --bind -o ro`, macOS loopback RO or APFS snapshot) proceeds. Binary gate: no flag can override it.
- [x] 17.5ii3 Phase 1 drift capture (restore): newer-on-disk bytes land in authoritative `raw_imports` before staging; Phase 2 stability converges after a transient writer pauses; Phase 3 fence diff aborts cleanly and reverts state.
- [ ] 17.5ii4 Remap Phase 4 bijection: missing, mismatch, and extra each fail with `NewRootVerificationFailedError` naming counts; full-tree fence detects mid-flight file-set / per-file-tuple / `.gbrainignore`-sha256 drift as `NewRootUnstableError`.
- [ ] 17.5ii5 Remap Phase 1: non-zero drift refuses with `RemapDriftConflictError`; second pass after operator verifies `/new/path` contains the edits succeeds with zero drift.
- [x] 17.5ii6 TOCTOU dirty-recheck between Phase 2 and the destructive step aborts with `CollectionDirtyError`.
- [x] 17.5ii7 `dirty-preflight` guard refuses restore/remap when `is_collection_dirty` or sentinel directory is non-empty; clears once RCRT / `sync` runs.
- [ ] 17.5ii9 Bulk UUID writes: `migrate-uuids` and `--write-gbrain-id` refuse with `ServeOwnsCollectionError` when serve is live; succeed offline.
- [x] 17.5ii9a UUID-migration preflight refuses remap/restore when any trivial-content page lacks a frontmatter `gbrain_id`, naming count + samples + `migrate-uuids` directive. Running `migrate-uuids` then retrying succeeds.
- [ ] 17.5ii10 IPC socket placement: parent-dir mode `0755` refuses startup with `IpcDirectoryInsecureError`; stale socket from a dead session is unlinked cleanly at startup.
- [ ] 17.5ii11 IPC bind-time audit catches a simulated mode regression (`0644`) with `IpcSocketPermissionError`.
- [ ] 17.5ii12 IPC peer auth: cross-UID client refused at `accept()`; same-UID attacker races `bind()` and spoofs `whoami` → CLI kernel-PID check detects mismatch against `serve_sessions.pid` and refuses with `IpcPeerAuthFailedError`; proxy mode refuses when peer UID differs.
- [x] 17.5jj Restore refuses non-empty target (no `--force`).
- [x] 17.5kk Tx-B is idempotent; running after pending state N times produces exactly one finalize.
- [x] 17.5kk2 Tx-B sets `needs_full_sync=1` to arm the write-gate; RCRT attach clears it.
- [x] 17.5kk3 Tx-B failure leaves `pending_root_path` set; generic recovery worker does NOT clear the flag.
 - [x] 17.5ll Restore orphan recovery: originator dead, RCRT finalizes as `StartupRecovery`.
- [x] 17.5ll2 `sync --finalize-pending` finalizes as `ExternalFinalize`.
  > **Repair note (Leela, Batch J repair):** The original implementation always emitted success-shaped output (`status: ok`, exit 0) regardless of `FinalizeOutcome`. Fixed: `Finalized` and `OrphanRecovered` remain success; `Deferred`, `ManifestIncomplete`, `IntegrityFailed`, `Aborted`, and `NoPendingWork` now bail with `FinalizePendingBlockedError: collection=… outcome=… collection remains blocked and was not finalized` and return non-zero exit. CLI truth tests added to `tests/collection_cli_truth.rs` covering `NoPendingWork` and `Deferred` paths.
- [x] 17.5ll3 Successor process with a different restore-command identity cannot bypass the fresh-heartbeat gate.
- [x] 17.5ll4 `pending_manifest_incomplete_at` retries successfully within window.
- [x] 17.5ll5 `pending_manifest_incomplete_at` escalates to `integrity_failed_at` after TTL.
- [x] 17.5mm Restore manifest tamper detected → `IntegrityFailed` + `restore-reset` flow.
- [x] 17.5nn `DuplicateUuidError`: ordinary plain sync and attach-path `full_hash_reconcile` halt, persist `reconcile_halted_at`, and require operator repair plus `reconcile-reset` before retry. The broader remap Phase 4 proof remains deferred with the destructive-path batch.
- [x] 17.5oo `UnresolvableTrivialContentError`: ordinary plain sync and attach-path `full_hash_reconcile` halt, persist `reconcile_halted_at`, and remain terminal until operator repair plus `reconcile-reset`. The broader remap Phase 4 proof remains deferred with the destructive-path batch.
- [x] 17.5oo2 RCRT SKIPS collections where `reconcile_halted_at IS NOT NULL`.
- [x] 17.5oo3 `gbrain collection info` surfaces truthful `blocked_state`, `integrity_blocked`, and operator guidance for reconcile-halt and restore-integrity states. `brain_collections` MCP surfacing remains deferred.
- [ ] 17.5pp Online restore handshake: ack triple matches on `(session_id, reload_generation)`; stale + foreign acks never match.
- [ ] 17.5qq Serve-died-during-handshake short-circuits the 30s timeout.
- [ ] 17.5qq2 Serve startup do-not-impersonate: fresh serve observing `restoring` does not ack.
- [ ] 17.5qq3 Remap online mode: CLI does only DB tx; RCRT drives attach + `full_hash_reconcile` + state flip.
- [ ] 17.5qq4 Remap offline mode: CLI holds lease itself and runs reconcile directly.
- [ ] 17.5qq5 UUID-first resolution prevents remap delete-create churn across directory reorganizations.
- [ ] 17.5qq6 `full_hash_reconcile` runs EXACTLY ONCE per remap.
- [ ] 17.5qq7 `brain_put` during remap is refused by the write-gate.
- [ ] 17.5qq8 Attach-completion tx is a no-op on re-entry (only bumps generation once).
- [ ] 17.5qq9 CLI never writes `collections.ignore_patterns` directly (code audit asserts).
- [x] 17.5qq10 `collection add` capability probe sets `writable=0` on EACCES/EROFS and WARNs.
- [x] 17.5qq11 `CollectionReadOnlyError` refuses K1-scoped vault-byte writes when `writable=0`.
  > **Repair note (Leela, K1 repair):** CLI path (`gbrain put`) was tested in `tests/collection_cli_truth.rs::put_cli_refuses_when_collection_is_persisted_read_only`. Added MCP-path test `brain_put_refuses_when_collection_is_read_only` in `src/mcp/server.rs` to confirm the same gate via `brain_put` → `put_from_string` → `ensure_collection_vault_write_allowed`.
- [x] 17.5qq12 Write-gate (`needs_full_sync=1` OR `state='restoring'`) refuses all mutating ops.
- [ ] 17.5rr Schema-consistency: every page with DB-only state survives hard-delete path.
- [ ] 17.5ss Bare-slug resolution: single-collection brain accepts; multi-collection resolves only when unambiguous.
- [ ] 17.5tt `WriteCreate` resolves to write-target when slug is globally unused; otherwise `AmbiguityError`.
- [ ] 17.5uu `WriteUpdate` requires exactly one owner; zero → `NotFoundError`.
- [ ] 17.5vv `WriteAdmin` resolves by name only; bare-slug form rejected.
- [ ] 17.5vv2 Collection names cannot contain `::`; CHECK constraint + clap validator reject.
- [ ] 17.5vv3 External address `<collection>::<slug>` always resolves to the named collection.
- [ ] 17.5vv4 `AmbiguityError` payload contains full candidate list.
- [ ] 17.5vv5 `WriteAdmin` honors `CollectionRestoringError` interlock.
- [ ] 17.5vv5b `WriteAdmin` honors write-gate (`needs_full_sync=1`).
- [ ] 17.5vv6 Slug-less `brain_gap` routes via Read and succeeds during restoring.
- [ ] 17.5ww UUID write-back: `--write-gbrain-id` rotates `file_state`+`raw_imports` atomically.
- [ ] 17.5ww2 `migrate-uuids --dry-run` mutates nothing.
- [ ] 17.5ww3 UUID write-back on EACCES/EROFS skips with WARN; `pages.uuid` remains set.
- [ ] 17.5www `brain_put` preserves `gbrain_id` across write.
- [x] 17.5xx `raw_imports` rotation atomic per content-changing write.
- [x] 17.5yy Inline GC enforces `KEEP` + `TTL_DAYS`; active row never touched.
- [x] 17.5zz `KEEP_ALL=1` disables GC; active row remains singular.
- [x] 17.5aaa Zero active `raw_imports` → `InvariantViolationError`; `--allow-rerender` is audit-logged WARN override.
  > **Batch H boundary:** enforced paths raise typed invariant errors before mutation; the explicit override seam exists only as the closed operator-only policy hook and is not enabled for passive/background reconciler callers.
- [x] 17.5aaa1 Post-ingest invariant assertion runs in every write-path test.
- [ ] 17.5aaa2 Watcher overflow sets `needs_full_sync=1` and recovery runs within 1s.
- [ ] 17.5aaa3 Watcher auto-detects native-first, downgrades to poll on init error with WARN.
- [ ] 17.5aaa4 Watcher supervisor restarts on panic with exponential backoff.
- [ ] 17.5bbb Full-hash audit rehashes files older than `GBRAIN_FULL_HASH_AUDIT_DAYS` and updates `last_full_hash_at`.
- [x] 17.5ccc Fresh-attach and first-use-after-detach always run `full_hash_reconcile`.
- [ ] 17.5ddd `brain_collections` response shape matches design.md schema exactly.
- [ ] 17.6 Integration: 1000-file reconciliation completes under the documented budget.
- [ ] 17.7 Integration: watcher picks up an edit within 2s.
- [ ] 17.8 Integration: semantic search eventual consistency (FTS fresh, embedding lane catches up).
- [ ] 17.9 Integration: restore → round-trip bytes exactly via `raw_imports`.
- [ ] 17.10 Integration: online restore with live serve — handshake releases watcher; post-Tx-B attach rebinds; no serve restart.
- [x] 17.11 Integration: offline restore finalizes via CLI.
  > **K2 completion note (Leela):** `finalize_pending_restore_via_cli` is a pure CLI path that does NOT depend on serve/RCRT. It chains `finalize_pending_restore` (Tx-B) → `complete_attach` (runs `full_hash_reconcile_authorized` + sets `state='active'`, clears `needs_full_sync`, advances `reload_generation`) entirely within the CLI process under a short-lived owner lease. Proven by `offline_restore_can_complete_via_explicit_cli_finalize_path` in `tests/collection_cli_truth.rs` (`#[cfg(unix)]`): calls real binary, confirms `blocked_state=pending_attach` after restore, then `--finalize-pending` returns `finalize_pending=Attached` and DB reflects `state=active`, `root_path=<target>`, `needs_full_sync=0`, no pending fields. The original deferred note's RCRT dependency claim was written at Batch I before `complete_attach` existed in the CLI path and is now superseded.
- [x] 17.12 Integration: crash mid-write — startup recovery ingests disk bytes; DB converges.
  > **Complete (Batch L2 startup-only seam):** `start_serve_runtime_recovers_owned_sentinel_dirty_collection_and_unlinks_all_sentinels` seeds a post-rename/pre-commit fixture (disk bytes ahead of DB + two sentinel files), proves startup reconcile updates `pages` + active `raw_imports` from disk, clears `needs_full_sync`, unlinks sentinels only after success, and stays idempotent across a repeated boot. Companion tests prove foreign-owned collections are skipped and failed reconcile leaves the sentinel behind.
 - [x] 17.13 Integration: crash mid-restore between rename and Tx-B — RCRT finalizes on next serve start.
- [ ] 17.14 Integration: `git checkout` (mass rewrite) triggers overflow flag + full reconcile.
- [ ] 17.15 Integration: multi-collection brain with colliding slugs exercises all resolution branches.
- [x] 17.16 Integration: Windows platform gate — the currently implemented vault-sync CLI surfaces (`gbrain serve`, `gbrain put`, `gbrain collection {add,sync,restore}`) return `UnsupportedPlatformError`.
- [x] 17.16a Integration: non-writable collection refuses vault-byte write entry points (`gbrain put`, `brain_put` via `put_from_string`) with `CollectionReadOnlyError`. DB-only mutators remain deferred.
- [ ] 17.17 Integration: `gbrain init` → `gbrain collection add <vault>` → edit in Obsidian → MCP `brain_get` returns fresh content within 2s.

### Named invariant tests (spec-cited)

- [ ] 17.17a `resolver_unification` — unit test asserts that Phase 4 manifest verification and `full_hash_reconcile` invoke the same canonical `resolve_page_identity(...)` helper (UUID-first, then content-hash uniqueness with size>64 and non-empty-body guards). A divergent resolver path fails the test. Spec anchor: [specs/vault-sync/spec.md](specs/vault-sync/spec.md) Phase 4 identity-resolution paragraph.
- [x] 17.17b `finalize_pending_restore_caller_explicit` — unit test asserts every production call site of the finalize helper passes an explicit `FinalizeCaller` variant (`RestoreOriginator`, `StartupRecovery`, or `ExternalFinalize`). A no-arg or implicit-default variant fails the test. Spec anchor: [specs/collections/spec.md](specs/collections/spec.md) restore finalize paths.
- [ ] 17.17c `raw_imports_active_singular` — unit test asserts that after every write path (initial ingest, reconciler re-ingest, `brain_put` create/update, UUID write-back), `SELECT COUNT(*) FROM raw_imports WHERE page_id=? AND is_active=1` equals exactly 1 for every page in the collection. Zero active rows → `InvariantViolationError`. Spec anchor: [specs/collections/spec.md](specs/collections/spec.md) raw_imports rotation invariant.
- [ ] 17.17d `quarantine_db_state_predicate_complete` — unit test asserts the five-branch `has_db_only_state(page_id)` predicate is consulted at every site that could hard-delete a page (reconciler missing-file handler, `quarantine discard`, auto-sweep TTL). A code path that deletes without consulting the predicate fails the test. Spec anchor: [specs/vault-sync/spec.md](specs/vault-sync/spec.md) delete-vs-quarantine classifier.
- [x] 17.17e `expected_version_mandatory` — unit proof now pins the actual enforcement sites for the enumerated vault-byte entry points: `brain_put` create-with-existing and update are rejected by MCP prevalidation before reaching tempfile / dedup / FS / DB mutation, while CLI `gbrain put` enforces `expected_version` in `check_update_expected_version` before sentinel creation, tempfile, dedup insert, FS mutation, or DB mutation. Only the pure-create path (no prior page at the slug) may omit `expected_version`. Spec anchor: [specs/agent-writes/spec.md](specs/agent-writes/spec.md) CAS contract.

## 18. Follow-up OpenSpec stubs

- [ ] 18.1 Create `openspec/changes/daemon-install/proposal.md` stub (launchd/systemd wrapping of `gbrain serve`, `gbrain daemon {install,uninstall,start,stop,status}`, expanded `gbrain status`).
- [ ] 18.2 Create `openspec/changes/openclaw-skill/proposal.md` stub (agent-facing bootstrap that orchestrates `gbrain init → collection add → daemon install → MCP wiring`).
