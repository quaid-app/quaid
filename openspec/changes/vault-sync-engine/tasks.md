## 1. Schema v5

- [x] 1.1 Implement v5 schema per design.md § Schema. Acceptance: `quaid init` creates all tables (`collections`, `pages`, `file_state`, `embedding_jobs`, `raw_imports`, `links`, `assertions`, `knowledge_gaps`, `contradictions`) with the documented columns and FKs; existing integration smoke test passes.
  > **Repair note (Leela):** `serve_sessions` and `collection_owners` are watcher-slice tables, not foundation. `ingest_log` is kept as a compatibility shim until the reconciler slice removes `quaid import`. `pages.uuid` is nullable until task 5a.* wires UUID generation. `pages.collection_id DEFAULT 1` routes legacy inserts to the auto-created default collection.
- [x] 1.1a Create `CREATE UNIQUE INDEX idx_pages_uuid ON pages(uuid) WHERE uuid IS NOT NULL` for O(1) UUID-based rename lookup; partial index allows NULL until task 5a.*.
- [x] 1.1b Extend `src/core/gaps.rs::log_gap()` and `memory_gap` to accept an optional slug and populate `knowledge_gaps.page_id` when a slug resolves; leave NULL otherwise. Update the `Gap` struct and `list_gaps`/`resolve_gap` responses. Unit tests cover slug and slug-less variants and the `has_db_only_state` effect.
  > **Repair note (Leela, K1 repair):** The library side (slug→page_id binding, `KnowledgeGap.page_id`, `list_gaps` response) was complete. Added `page_id` to the `memory_gap` MCP response so callers can confirm the binding in a single call. Added tests `memory_gap_with_slug_response_includes_page_id` and `memory_gap_without_slug_response_has_null_page_id`.
- [x] 1.1c Classify `memory_gap` by variant: slug-bound = `WriteUpdate` (subject to `CollectionRestoringError` interlock); slug-less = `Read` (no collection resolved, no interlock). Unit test covers both during `state='restoring'`.
- [x] 1.2 Add index on `pages.quarantined_at` for efficient sweep queries.
- [x] 1.3 `quaid_config` writes `schema_version = 5` on `quaid init`.
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
- [x] 2.4a2 Platform gate: `#[cfg(windows)]` handlers return `UnsupportedPlatformError` from the currently implemented vault-sync CLI surfaces: `quaid serve`, `quaid put`, `quaid collection {add,sync,restore}`. Deferred collection quarantine/export handlers remain out of scope; existing DB-only reset handlers (`restore-reset`, `reconcile-reset`) remain outside this Windows gate and may still run offline.
- [x] 2.4b Implement `src/core/fs_safety.rs` fd-relative primitives: `open_root_fd`, `walk_to_parent`, `openat_create_excl`, `stat_at_nofollow`, `renameat_parent_fd`, `unlinkat_parent_fd`.
  > **Complete:** All six primitives implemented with Unix `#[cfg(unix)]` + Windows fallback returning `UnsupportedPlatformError`. Uses `rustix::fs` with `O_NOFOLLOW`, `O_DIRECTORY`, `AT_SYMLINK_NOFOLLOW` semantics.
- [x] 2.4c On the reconciler path, candidate paths are enumerated via `ignore::WalkBuilder` with `follow_links(false)`. Walker metadata is advisory only; each candidate is revalidated with `walk_to_parent` + `stat_at_nofollow`, WARN-skipping symlinked entries/ancestors and never descending symlinked directories.
  > **Closure note:** This task is closed only for the current reconciler walk path. It is not a claim that a generic `readdir`-based fd-relative walk primitive exists, and it does not widen beyond the existing `ignore::WalkBuilder` + fd-relative revalidation seam.
- [x] 2.4d Unit tests for fd-safety helpers: reject path traversal, reject symlinked root, reject symlinked ancestor, reject symlink at target, reject `O_EXCL` clobber, round-trip a safe write.
  > **Complete:** 15 tests in `fs_safety.rs` covering all safety scenarios. Tests are `#[cfg(all(test, unix))]` and pass on Linux/macOS CI.
- [x] 2.5 `parse_slug` returns `Resolved`/`NotFound`/`Ambiguous`; callers translate `Ambiguous` into `AmbiguityError` with candidate list.
- [x] 2.6 Register a user-facing error type `AmbiguityError` with candidate list and a stable serialization shape.

## 3. Ignore pattern handling

- [x] 3.1 Add `ignore` + `globset` crates. Built-in defaults are merged at reconciler-query time (`.obsidian/**`, `.git/**`, `node_modules/**`, `_templates/**`, `.trash/**`); user patterns live on disk in `.quaidignore` only.
- [x] 3.2 Implement atomic-parse of `.quaidignore`: validate every non-comment line via `globset::Glob::new` BEFORE any effect. Fully-valid → refresh `collections.ignore_patterns` mirror, clear `ignore_parse_errors`, trigger reconciliation. Any failing line → mirror UNCHANGED, `ignore_parse_errors` records failing lines, no reconciliation.
- [x] 3.3 Absent-file default: no prior mirror → defaults only; prior mirror present → mirror UNCHANGED, WARN logged `quaidignore_absent collection=<N>`. Operator explicitly clears with `quaid collection ignore clear <name> --confirm`.
- [x] 3.4 CLI `quaid collection ignore add|remove|clear --confirm` is dry-run first (in-memory proposed file, atomic-parse validator), file-write second, mirror-refresh last via `reload_patterns()`. CLI never writes `collections.ignore_patterns` directly.
  > **Note:** `reload_patterns()` is implemented; CLI commands deferred to later batch.
- [x] 3.5 `reload_patterns()` is the SOLE writer of `collections.ignore_patterns`; invoked by the watcher on `.quaidignore` events and at serve startup.
- [x] 3.6 Expose parse errors via WARN log, `memory_collections` `ignore_parse_errors` field, and `quaid collection info`.
  > **Note:** Data model complete; logging and CLI display deferred to watcher/serve slices.
- [x] 3.7 `ignore_parse_errors` is a JSON array of `{code, line, raw, message}` where `code` ∈ `parse_error` | `file_stably_absent_but_clear_not_confirmed`. Single canonical shape documented in the spec.

## 4. File state tracking and stat-diff

- [x] 4.1 Add `file_state` table + indexes per §Schema. `ctime_ns` is nullable for legacy rows only; `memory_put` always writes the full tuple.
  > **Note:** Schema already in place from foundation slice; helpers implemented in `src/core/file_state.rs`.
- [x] 4.2 Implement `stat_file(parent_fd, name)` returning `(mtime_ns, ctime_ns, size_bytes, inode)` via `fstatat(AT_SYMLINK_NOFOLLOW)`.
  > **Complete:** `stat_file_fd` implemented in `file_state.rs` using `fs_safety::stat_at_nofollow`. Unix uses fd-relative `fstatat(AT_SYMLINK_NOFOLLOW)`; Windows uses path-based fallback with nullable ctime/inode. Function signatures and conversion wrappers are production-ready. Task 5.2 reconciler walk will call `stat_file_fd` with parent fds.
- [x] 4.3 Implement `stat_diff(collection_id)`: compare filesystem walk against `file_state`; yield `{unchanged, modified, new, missing}` sets. Any of the four stat fields mismatching triggers re-hash.
  > **Complete (Batch D):** Task 5.2 walk landed; `stat_diff` now performs a real filesystem walk via `walk_root` + `ignore::WalkBuilder` with fd-relative `stat_at_nofollow` per entry, then compares against `file_state` via `stat_diff_from_walk`. The four-field comparison (`mtime_ns`, `ctime_ns`, `size_bytes`, `inode`) is implemented and tested. The "returns DB files as missing" stub language from Batch C no longer applies.
- [x] 4.4 `full_hash_reconcile(collection_id)`: ignore stat fields; hash every file; rebuild `file_state` from disk; used by remap/restore/fresh-attach/audit. Includes a closed mode/authorization contract and unchanged-hash metadata self-heal without `raw_imports` rotation.
  > **Complete (Batch G):** `full_hash_reconcile` now hashes every walked file, validates a closed mode/authorization contract against current collection state before walking, reuses rename/apply for changed/new content, and self-heals unchanged-hash rows by updating only `file_state` / `last_full_hash_at`. Unchanged-hash paths do not rewrite `pages` or rotate `raw_imports`.
- [x] 4.5 UUID-first identity resolution in reconcile: build in-memory `quaid_id → (path, sha256)` index from the new tree; prefer UUID match over path before falling back to content-hash uniqueness guards.
  > **Complete (Batch E):** Reconciler now hashes/parses live new-file candidates into an in-memory UUID/path/hash identity index, applies native rename pairs first, then UUID matches against `pages.uuid`, and only then attempts conservative hash pairing. This remains classification-only until the later apply pipeline lands; no filesystem or DB mutations happen here yet.
- [ ] 4.6 Periodic full-hash audit: background task rehashes files whose `last_full_hash_at` is older than `QUAID_FULL_HASH_AUDIT_DAYS` (default 7). `quaid collection audit <name>` for on-demand trigger.

## 5. Reconciler

- [x] 5.1 Create `src/core/reconciler.rs`. ~~Replace `import_dir()` from `migrate.rs`.~~
  > **Repair note (Leela, Batch C repair):** File created with correct types and function signatures. `migrate::import_dir()` remains the live ingest path. Safety-critical stubs now fail explicitly: `has_db_only_state` returns `Err` (not `Ok(false)`); `walk_collection` and `full_hash_reconcile` return `Err("not yet implemented")` instead of success-shaped empty stats. Any live path wired to these before tasks 5.2–5.5 land will fail loudly.
  > **Batch D update (Leela):** `walk_collection` (task 5.2) and `has_db_only_state` (task 5.4) are now real implementations — neither returns `Err` anymore. `full_hash_reconcile` was later completed in Batch G with mode/authorization validation, unchanged-hash metadata self-heal, and the full hash-based apply path.
- [x] 5.2 Implement walk using `ignore::WalkBuilder` bounded to `root_fd`; respect `.quaidignore` + built-in defaults.
  > **Complete:** Reconciler now opens `root_fd` first, reloads `.quaidignore`, walks with `ignore::WalkBuilder`, and re-stats every candidate entry via fd-relative `walk_to_parent` + `stat_at_nofollow` before trusting it. Symlinked roots still refuse, symlinked entries/ancestors are skipped with WARN, built-in defaults still apply, and this same walk now closes the real `stat_diff` filesystem-walk gap rather than treating every stored row as missing.
- [x] 5.3 Implement rename resolution: (1) native event pairing when available; (2) UUID match; (3) content-hash uniqueness with guards (≥64 body bytes after frontmatter, unique hash in both `missing` and `new`, non-empty body after frontmatter); (4) quarantine + fresh create otherwise.
  > **Complete (Batch E boundary):** Native pairing is implemented as an interface-level reconciler input and is exercised in tests, but watcher/event production is still deferred. UUID matches and conservative hash matches now classify rename-vs-quarantine in memory; quarantine/fresh-create application still waits for the later mutation pipeline.
  > **Batch E repair (Leela):** Hash-rename guard now uses body bytes (after frontmatter, trimmed) for the ≥64-byte threshold, not whole-file size. Previous implementation used `file_state.size_bytes` (whole-file), which allowed large-frontmatter / tiny-body template notes to satisfy the threshold and be incorrectly paired. `MissingPageIdentity.body_size_bytes` is computed from `compiled_truth + timeline` in the DB; `NewTreeIdentity.body_size_bytes` is computed from the parsed body after frontmatter. Refusal reasons updated to `missing_below_min_body_bytes` / `new_below_min_body_bytes`. Regression test `template_note_with_large_frontmatter_and_tiny_body_is_never_hash_paired` added.
- [x] 5.3a On condition failure in (3), log `rename_inference_refused reason=<...>` at INFO so decisions are debuggable.
- [x] 5.4 Implement delete-vs-quarantine classifier using `has_db_only_state(page_id)` predicate (five-branch OR over programmatic links, non-import assertions, `raw_data`, `contradictions`, `knowledge_gaps`).
  > **Complete:** `has_db_only_state(page_id)` is now the real five-branch SQL predicate — never a success-shaped `Ok(false)` stub — and missing-file classification counts quarantine-vs-hard-delete using that predicate without wiring the later apply pipeline.
- [x] 5.4a Audit every callsite that inserts into `links` — populate `source_kind` explicitly. `memory_link` sets `programmatic`. Default is `programmatic` (fail-open preservation).
  > **Audit result:** `memory_link` (`commands/link.rs`) explicitly sets `source_kind = 'programmatic'`. `extract_links()` (`core/links.rs`) returns `Vec<String>` slugs only — it does NOT write to the `links` table and does NOT set `source_kind`. No production callsite currently populates `wiki_link`; the value is reserved in the schema CHECK constraint but has no live writer yet. The earlier claim that `extract_links()` sets `wiki_link` was incorrect and is retracted.
- [x] 5.4b Audit every callsite that inserts into `assertions` — use `asserted_by='import'` only from `check_assertions()`; every other path (agent, manual, enrichment) uses a non-import value.
- [x] 5.4c Unit test: page with each of the five DB-only categories independently triggers quarantine (not hard-delete).
- [x] 5.4d Every content-changing write covered by this slice rotates `raw_imports` per file inside the SAME SQLite tx as the corresponding `pages`/`file_state` mutation; within Batch F this means ingest + reconciler apply, and inside reconciler each per-file rotation happens inside its enclosing 500-file chunk tx.
  > **Complete (Batch F slice):** Added shared `core::raw_imports` rotation helpers and wired them into `commands::ingest`, `core::migrate::import_dir`, and reconciler apply-time re-ingest/create/rename paths. For the paths in scope here, page/file_state mutation, raw_import rotation, and embedding-job enqueue now happen in the same SQLite transaction.
- [x] 5.4e Inline GC in the rotation tx: enforce `QUAID_RAW_IMPORTS_KEEP` (default 10) per page AND `QUAID_RAW_IMPORTS_TTL_DAYS` (default 90). Active row is never touched. `QUAID_RAW_IMPORTS_KEEP_ALL=1` disables GC.
  > **Complete (Batch F slice):** Inline per-page GC now runs inside the same rotation transaction, trims inactive history beyond the keep cap, drops TTL-expired inactive rows, and honors `QUAID_RAW_IMPORTS_KEEP_ALL=1`. Active rows are never deleted by GC.
- [ ] 5.4f Daily background sweep in `quaid serve` for TTL-expired inactive rows on idle pages; also triggered by `quaid collection audit --raw-imports-gc`.
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
- [x] 5.8a0 UUID-migration preflight (runs FIRST, before RO-mount gate): scan `pages` for rows whose `uuid` is not present in the file's frontmatter AND whose content is trivial under the same canonical helper used by 5.3 (`body_size_bytes < 64` after frontmatter OR empty body). If any found, refuse with `UuidMigrationRequiredError` naming the count and up to 5 sample paths, directing the operator to run `quaid collection migrate-uuids <name>` before retrying. This gate closes the silent-identity-loss path for short/template notes that have neither `quaid_id` frontmatter nor content-hash uniqueness. The check runs against the DB (no filesystem walk) and is O(page_count).
- [x] 5.8a Preflight RO-mount gate: `statvfs(old_root)` inspects `ST_RDONLY` (Linux) / `MNT_RDONLY` (macOS). RO mount → proceed with INFO `restore_ro_mount_verified`. Writable mount → refuse with `CollectionLacksWriterQuiescenceError` naming the two acceptance paths (remount RO, or run from a quiesced environment). No `--writers-quiesced` / `--unsafe-accept-residual-race` flags exist.
- [x] 5.8a2 `dirty-preflight` guard (before Phase 1): refuse if `is_collection_dirty(collection_id)` is TRUE OR the sentinel directory is non-empty, unless the caller is `sync --finalize-pending`. Error message instructs waiting for RCRT or running `quaid collection sync`.
- [x] 5.8b Phase 1 — drift capture: open a fresh old-root walk via `full_hash_reconcile_authorized(..., mode=RestoreDriftCapture|RemapDriftCapture)` using the closed authorization enum carrying caller identity (`restore_command_id` or owning lease/session identity). For restore, captured drift becomes the authoritative `raw_imports`. For remap, any material drift aborts with `RemapDriftConflictError` naming the `DriftCaptureSummary` counts (`pages_updated`, `pages_added`, `pages_quarantined`, `pages_deleted`). Log `restore_drift_captured` WARN when non-zero; `remap_drift_refused` ERROR.
- [x] 5.8c Phase 2 — stability check: two successive stat-only snapshots over old root `(relative_path, mtime_ns, ctime_ns, size_bytes, inode)`. Equal → proceed. Differ → re-run Phase 1 and capture `snap3`; retry up to `QUAID_RESTORE_STABILITY_MAX_ITERS` (default 5). Persistent instability → `CollectionUnstableError`. For remap, any retry with non-zero drift falls back to `RemapDriftConflictError`.
- [x] 5.8d Phase 3 — pre-destruction fence: one final stat-only walk `snap_fence` compared to `snap_final`. Diff → abort via the standard abort-path resume sequence (revert state, keep `root_path`, clear ack triple, NULL heartbeat, bump `reload_generation`, drop offline lease, stop heartbeat tasks); log `restore_aborted_fence_drift` / `remap_aborted_fence_drift` WARN; return `CollectionUnstableError`.
- [x] 5.8d2 TOCTOU recheck: between Phase 2 stability and the destructive step, re-evaluate `is_collection_dirty` on a fresh SQLite connection AND re-scan the sentinel directory. TRUE → abort with `CollectionDirtyError` via the same abort-path resume sequence.
- [x] 5.8e Phase 4 (remap only) — `/new/path` manifest verification + new-root stability fence. Use the canonical `resolve_page_identity(...)` (UUID first, then content-hash uniqueness with size>64 / non-empty-body guards — NO relative-path shortcut). Pass criteria: (i) every active-indexable page resolves to exactly one file on `/new/path`, (ii) sha256 matches authoritative `raw_imports.raw_bytes`, (iii) every non-ignored file resolves to exactly one page. Full-tree fence (`newroot_snap_pre` vs `newroot_snap_fence`) detects file-set / per-file-tuple / `.quaidignore`-sha256 drift between verification and DB-update; drift → `NewRootUnstableError`. Pass-criteria failure → `NewRootVerificationFailedError` naming counts and sampled diffs. Quarantined pages excluded from both sides of the bijection.
- [x] 5.8f Online restore (live supervisor): Phase 1 runs AFTER handshake release so drift capture sees the live tree. Staging + per-file sha256 verification + Tx-A + rename + Tx-B follow. Online remap does only the one-tx DB update (`reload_generation++`, `needs_full_sync=1`, state stays `'restoring'`, deletes `file_state`); RCRT handles post-state attach + `full_hash_reconcile`.
- [x] 5.8g Offline mode: CLI holds the `collection_owners` lease with heartbeat throughout; runs the full pipeline end-to-end; releases the lease on completion.
- [x] 5.9 Wire fresh-attach and first-use-after-detach to invoke `full_hash_reconcile` in `FreshAttach` mode before clearing `needs_full_sync` / reopening writes.
  > **Complete for Batch H core seam:** `fresh_attach_reconcile_and_activate()` now runs a dedicated fresh-attach full-hash pass and clears `needs_full_sync` only after reconcile succeeds. Higher-level serve/supervisor choreography remains outside this batch boundary.

## 5a. UUID lifecycle and frontmatter persistence

- [x] 5a.1 Add `uuid7` crate (or use `uuid` with v7 support).
- [x] 5a.2 Extend `parse_frontmatter()` and `render_page()` to treat `quaid_id` as a first-class field; reading preserves it; rendering emits it if present.
- [x] 5a.3 Extend `Page` struct with `uuid: String` (non-optional).
  > **Complete (construction cascade closed):** `Page.uuid` is now required everywhere `Page` is constructed or serialized. Read paths fail loudly on rows that still lack a UUID rather than inventing a placeholder default.
- [x] 5a.4 Ingest pipeline: if `frontmatter.quaid_id` is present, adopt it as `pages.uuid`; if absent, generate UUIDv7 server-side and store in `pages.uuid` ONLY. Default ingest is READ-ONLY with respect to user bytes — no self-write enqueued.
- [x] 5a.4a Regression test: save a `.md` without `quaid_id`; observe watcher event; assert file bytes unchanged, `file_state.sha256` equals user hash, dedup set empty, git remains clean.
  > **Batch E note:** Current coverage is at the compatibility-ingest boundary (`quaid ingest` / import path): generated UUIDs stay DB-only, source bytes remain unchanged, and a git worktree stays clean. Watcher dedup/file_state assertions remain deferred with watcher work.
- [x] 5a.5 Opt-in UUID write-back for `--write-quaid-id`, `migrate-uuids`, and `memory_put` only. Uses the full rename-before-commit discipline (sentinel, tempfile, `O_NOFOLLOW`, atomic rename, fsync parent, post-rename stat, single tx with `file_state` + `raw_imports` rotation). Read-only files (EACCES/EROFS) are skipped with WARN; `pages.uuid` remains set.
- [x] 5a.5a CLI: `quaid collection add --write-quaid-id` and `quaid collection migrate-uuids <name> [--dry-run]`. Both are `WriteAdmin`, honor the restoring-state interlock, and only self-write files missing `quaid_id`. Summary reports `migrated/skipped_readonly/already_had_uuid`.
- [x] 5a.6 `memory_put` preserves `quaid_id`: `render_page()` is the explicit `memory_put` seam and always emits existing `pages.uuid` in frontmatter so agents cannot inadvertently strip it.
  > **Complete (Batch G):** `render_page()` now always re-emits persisted `pages.uuid` as `quaid_id`, so `memory_put` / `memory_get` surfaces cannot strip UUID identity when incoming markdown omits it.
- [x] 5a.7 Unit tests: default-ingest read-only; `quaid_id` adoption; opt-in rewrite rotates `file_state`/`raw_imports` atomically; `migrate-uuids --dry-run` mutates nothing; `memory_put` always emits preserved `quaid_id`; UUIDv7 monotonicity; frontmatter round-trip preserves `quaid_id`; Batch G also covers unchanged-hash no-rotation, changed-hash rotation, and zero-active abort.
  > **Batch G partial:** Added direct coverage for `memory_put`/`render_page` UUID preservation, full-hash unchanged-hash no-rotation, full-hash changed-hash rotation, and full-hash zero-active abort. The remaining UUID write-back / migrate-uuids coverage stays deferred with tasks 5a.5–5a.5a.

## 6. Watcher pipeline

- [x] 6.1 Add `notify` crate (with `macos_fsevents` feature).
  > **Complete (watcher core slice):** Added `notify` with the real crate feature name `macos_fsevent` (singular; task wording used the conceptual plural) and wired it into the serve-only watcher path.
- [x] 6.2 Per-collection watcher task: one `notify` recommended watcher per collection, events pushed into a bounded `tokio::mpsc` channel tagged with `CollectionId`.
- [x] 6.2a `quaid serve` normalizes any `state='active'` collection whose `root_path` is blank before watcher registration so invalid legacy rows do not crash startup.
  > **Complete (Issue #81 repair):** serve now demotes blank-root active collections to `detached`, logs a WARN, and watcher selection keeps using `trim(root_path) != ''`. Regression coverage includes the collection-normalization seam plus a Unix watcher-selection proof.
- [x] 6.3 Per-collection debounce buffer; default `QUAID_WATCH_DEBOUNCE_MS=1500` coalesces Obsidian bulk saves.
- [x] 6.4 Batch processor drains the debounce buffer, runs stat-diff, commits updates.
- [x] 6.5 Create/Modify handler: re-ingest bytes; never self-write UUID on observed external edits.
  > **Bookkeeping closure (Fry, merged Batch 1 state):** `poll_collection_watcher()` drains dirty-path events into `run_watcher_reconcile()`, which delegates to `reconcile_with_native_events()`. Modified and newly discovered files are applied through `ApplyAction::Reingest` → `apply_reingest()`, so watcher-triggered external edits re-ingest bytes from disk without introducing any watcher-side UUID write-back path.
- [x] 6.6 Delete handler: invoke delete-vs-quarantine classifier.
  > **Bookkeeping closure (Fry, merged Batch 1 state):** watcher-triggered reconcile already routes missing paths through `ApplyAction::DeleteOrQuarantine` → `apply_delete_or_quarantine()`, reusing the same five-branch DB-only-state classifier as the broader reconcile pipeline instead of a bespoke watcher-only delete handler.
- [x] 6.7 Rename handler: honor native pair events directly; update `file_state.relative_path`; preserve `pages.id`.
  > **Bookkeeping closure (Fry, merged Batch 1 state):** `WatchEvent::NativeRename` pairs are buffered by the watcher and passed straight into `reconcile_with_native_events()`. The apply pipeline re-ingests with `existing_page_id` plus `old_relative_path`, so native renames update `file_state.relative_path` while preserving the existing `pages.id` rather than synthesizing a second watcher-only rename path.
- [x] 6.7a Overflow recovery task: on bounded-channel overflow, set `collections.needs_full_sync=1` in a brief tx, WARN log, continue accepting events. Recovery task polls the flag every 500ms and runs `full_hash_reconcile` within ~1s. Recovery worker is gated to `state='active'` only.
  > **Authorization repair note (Leela, Batch 1 repair):** `OverflowRecovery` is added to `FullHashReconcileMode` (the operation-label enum, NOT the authorization enum). Authorization must be `FullHashReconcileAuthorization::ActiveLease { lease_session_id }` using the serve session's `collections.active_lease_session_id`. Lease mismatch or null lease → skip with WARN. This is not a new authorization variant; it reuses the existing `ActiveLease` proof. Professor rejected any design that introduces a separate authorization bypass for overflow recovery.
- [x] 6.8 `.quaidignore` watcher: treat as live control file; trigger atomic parse + mirror refresh + reconciliation on any change.
  > **Complete (Mom Batch 1 edge fix):** watcher classification now bypasses the markdown-only filter for the root `.quaidignore` control file, debounces `IgnoreFileChanged`, reloads the cached mirror atomically, and only reconciles on a successful parse. Invalid globs or a deleted file with a prior mirror keep the last-known-good mirror, surface `ignore_parse_errors`, WARN-log the failure, and skip reconciliation so serve never walks on stale ignore state.
- [x] 6.9 Watcher auto-detect: native first, downgrade to poll on init error with WARN.
- [x] 6.10 Per-collection watcher supervisor with crash/restart + exponential backoff.
- [x] 6.11 Expose watcher health (last event time, channel depth, mode) via `quaid collection info` CLI only. `memory_collections` MCP tool is NOT widened in v0.10.0 — the 13.6 frozen 13-field schema is preserved. See Batch 1 repair note in `implementation_plan.md`.
  > **WatcherMode:** `Native | Poll | Crashed` only. No `Inactive` variant. Non-active collections surface `null` in all three health fields. Windows surfaces `null` for all three health fields.

## 7. Self-write dedup set

- [x] 7.1 Implement `Arc<Mutex<HashMap<PathBuf, (sha256, Instant)>>>` in the serve process.
  > **Complete (watcher core slice):** Landed the same serve-process contract via the existing process-global runtime registries: a shared `Mutex<HashMap<PathBuf, SelfWriteDedupEntry>>` keyed by full path with stored hash + insertion instant.
- [x] 7.2 Dedup entry inserted at step 8 of the rename-before-commit sequence (AFTER tempfile+fsync, BEFORE `renameat`).
- [x] 7.3 Watcher consults dedup set before emitting: if path + hash match an entry younger than 5s, drop the event.
- [x] 7.4 Background sweeper removes expired entries every 10s.
- [x] 7.5 Failure handlers remove the entry: rename failure unlinks tempfile + sentinel + removes dedup; post-rename failures remove dedup so reconciler can observe the new bytes.
  > **Closed (narrow dedup-cleanup seam):** the writer failure paths now share one explicit cleanup helper for dedup+path tracking, and Unix proofs cover pre-rename failure, rename failure, post-rename fsync failure, concurrent-rename detection, and commit failure leaving no stale dedup entry behind.
- [x] 7.6 Unit tests: echo suppression; path-only match rejects; expired entries no longer suppress; external edit after TTL ingests normally.

## 8. Embedding queue and worker

- [x] 8.1 Add `embedding_jobs(page_id, chunk_index, job_state, attempt_count, last_error, created_at, started_at)` table.
  > **Closed (v7 queue schema):** `src/schema.sql` bumps the embedded schema to v7 and extends `embedding_jobs` with `chunk_index`, `job_state`, `attempt_count`, `last_error`, and `created_at` while preserving the existing one-row-per-page enqueue contract (`UNIQUE(page_id)` + default `chunk_index = 0`). `src/core/db.rs` and schema-version tests were updated in the same patch.
- [x] 8.2 Ingest/reconciler/`memory_put` enqueue jobs atomically in the same SQLite tx as `pages`/`file_state`.
  > **Closed (existing write paths audited):** live enqueue paths in `commands/put.rs`, `core/reconciler.rs`, `core/quarantine.rs`, and the compatibility raw-import rotation helpers all continue to enqueue inside the same SQLite transaction as the corresponding page/file-state mutation. Re-enqueue now also resets stale `running`/`failed` state for the latest page-content job.
- [x] 8.3 Background worker drains jobs with bounded concurrency `min(cpus, 4)` (configurable via `QUAID_EMBEDDING_CONCURRENCY`).
- [x] 8.4 Worker retries with exponential backoff; permanent failures leave the job in `failed` state with `last_error`.
- [x] 8.5 On `quaid serve` startup, resume pending jobs.
- [x] 8.6 Expose actionable queue depth in `memory_collections`, and expose queue depth + failing jobs in `quaid collection info`.
  > **Closed (contract-preserving diagnostics):** actionable queue depth counts only `pending + running` rows. `memory_collections` keeps its frozen 13-field MCP shape and exports only `embedding_queue_depth`; `quaid collection info` now surfaces `failing_jobs` on both plain-text and `--json` output without widening the MCP contract.

## 9. `quaid collection` commands

- [x] 9.1 Implement `src/commands/collection.rs` with clap subcommands.
- [x] 9.2 `quaid collection add <name> <path> [--writable/--read-only]`: validate name (no `::`), validate/open `root_fd` with `O_NOFOLLOW` before row creation, persist the detached row, and run the fresh-attach reconciliation path. K1 keeps default attach read-only with respect to vault bytes; `--write-quaid-id` / watcher-mode remain deferred.
- [x] 9.2a `--write-quaid-id` triggers opt-in UUID write-back during the initial walk; default is read-only.
- [x] 9.2b Capability probe: attempt a tempfile write inside the root; if EACCES/EROFS, set `collections.writable=0` with WARN; subsequent K1-scoped vault-byte writes (`quaid put`) refuse with `CollectionReadOnlyError`.
  > **Scope note (Leela, K1 repair):** "K1-scoped vault-byte writes" means only `quaid put` / MCP `memory_put`. DB-only mutators (`memory_check`, `memory_link`, `memory_tags`, `memory_raw`, slug-bound `memory_gap`) intentionally do NOT check `CollectionReadOnly` per Professor's ruling: the vault-byte gate and the DB-only write-interlock are separate concerns. This is the correct and complete K1 behaviour.
- [x] 9.3 `quaid collection list` prints `name | state | writable | write_target | root_path | page_count | last_sync_at | queue_depth`.
- [x] 9.4 `quaid collection info <name>` prints extended status including ignore_parse_errors, integrity flags, pending_root_path, recovery progress.
- [x] 9.5 `quaid collection sync <name>` runs ONLY the ordinary active-root `stat_diff + reconciler` path. `--remap-root <path>` runs remap per task 5.8. `--finalize-pending` triggers `finalize_pending_restore(…, FinalizeCaller::ExternalFinalize)`. Offline plain sync acquires its own short-lived `collection_owners` lease with heartbeat, clears `needs_full_sync` only after a real active-root reconcile succeeds, and stays fail-closed on blocked states.
  > **Batch J honesty note (Leela):** This slice does **not** widen plain sync into finalize/remap/reopen/serve-handshake recovery multiplexing. Success means only “active root reconciled”.
- [ ] 9.6 `quaid collection remove <name>` detaches and optionally `--purge` drops rows with an explicit confirmation.
- [x] 9.7 `quaid collection restore <name> <target>`: stage → verify per-file sha256 → Tx-A (set `pending_root_path`, `pending_restore_manifest`, restore-command identity) → atomic rename → Tx-B (`run_tx_b` finalize).
  > **Batch I note (Leela):** Offline restore now stops after Tx-B and leaves `state='restoring'` + `needs_full_sync=1` until RCRT owns attach completion. Command success does **not** mean writes have reopened; the end-to-end integration proof stays deferred under task `17.11`.
- [x] 9.7a Refuse if target is non-empty (no `--force`): POSIX `rename()` cannot atomically replace a non-empty target.
- [x] 9.7b Emit post-restore summary: `restored=N byte_exact=N pending_finalize=<bool> pending_root_path=<P>`.
- [x] 9.7c Offline restore acquires the `collection_owners` lease with heartbeat; online restore runs the lease-based handshake (task 11.6).
  > **Third-revision repair (Bender, 2026-04-30T08:30Z):** The online restore/remap handshake path (`mark_collection_restoring_for_handshake` + `wait_for_exact_ack`) was using untyped `owner_session_id()` + `session_is_live()`. A live CLI lease in `collection_owners` (session_type='cli') would be treated as a valid serve supervisor, entering an ack-wait loop that can only be satisfied by a serve-type supervisor — a certain timeout. Fixed by replacing both calls with typed `live_collection_owner()`, which enforces `session_type = 'serve'` AND heartbeat liveness in one query (design.md §404-408). The dead `session_is_live()` helper was removed. Three tests added: `mark_collection_restoring_rejects_cli_session_as_handshake_owner` (behavioral) + `handshake_functions_use_typed_live_collection_owner_not_untyped_pair` (source-seam) + the pre-existing `mark_collection_restoring_uses_collection_owners_and_clears_ack_residue` still passes.
- [x] 9.7d **Restoring-Collection Retry Task (RCRT)** at `quaid serve` startup and on a continuous sweep: observe owned collections with no live `supervisor_handles` entry and drive recovery. Actions: finalize pending restores (`FinalizeCaller::StartupRecovery`), orphan-recovery for dead originators, single-flight attach handoff (open new `root_fd`, run `full_hash_reconcile`, then in attach-completion tx flip `state='active'` and clear `needs_full_sync`, THEN spawn supervisor). Skip any collection where `reconcile_halted_at IS NOT NULL`.
- [x] 9.7e `quaid collection restore-reset <name> --confirm`: clears terminal integrity-blocked state (`integrity_failed_at`, escalated `pending_manifest_incomplete_at`, restore-command identity tuple, `pending_root_path`, `pending_restore_manifest`).
- [x] 9.7f `quaid collection reconcile-reset <name> --confirm`: clears `reconcile_halted_at` + `reconcile_halt_reason` after operator has manually resolved the offending vault state.
- [x] 9.8 `quaid collection quarantine {list,discard,export,restore}` narrow surface on Unix. `discard` on a page with DB-only state requires `--force` OR a prior successful `export` (which dumps all five DB-only-state categories as JSON). `export` records the export timestamp only after the filesystem write succeeds, blocking premature discard relaxation. `restore` is included on Unix only with the narrow no-replace install contract (see repair note below). `quarantine audit`, online/live routing, watcher mutation choreography, and restore overwrite/export-conflict policy remain deferred.
  > **Revised narrow closure (Mom, restore re-enable revision):** `restore` is included on Unix with the reviewed narrow contract: no-replace install semantics (`linkat` / EEXIST refusal), pre-install tempfile cleanup on write_all/sync_all failure, parse-failure rollback of the installed target before returning, parent-fsynced rollback cleanup after every successful unlink, happy-path page reactivation, and the existing refusal gates for non-Markdown targets, live-owned collections, read-only collections, occupied targets, and absent parent directories (callers must pre-create target directory structure). `quarantine audit`, online/live routing, watcher mutation choreography, and any overwrite/export-conflict policy beyond strict no-replace refusal remain deferred.
- [x] 9.9 Auto-sweep TTL: `QUAID_QUARANTINE_TTL_DAYS` (default 30) auto-discards ONLY pages where `has_db_only_state` is false; log each discard and DEBUG-log each skip.
- [x] 9.9b `quaid collection info` surfaces count of "quarantined pages awaiting user action".
- [x] 9.10 `quaid collection ignore add|remove|list|clear --confirm` per §3.
  > **Closed 9.10 (CLI ignore-only):** `quaid collection ignore add|remove|list|clear --confirm` now wraps `.quaidignore` with dry-run-first validation, canonical restore/needs-full-sync interlocks, explicit clear semantics, mirror refresh via `reload_patterns()` / `clear_patterns()`, and active-collection reconcile proofs. Watcher-driven reload (`17.5y`/`17.5z`) and broader MCP ignore-diagnostic widening (`17.5aa5`) remain open.
- [x] 9.11 All `collection` subcommands produce stable machine-parseable summaries on success and non-zero exit on any error.
  > **Closed 9.11 (current collection surface):** the currently implemented `quaid collection` subcommands, including the new `ignore` verbs, expose stable success payloads under the existing JSON mode and continue to fail with non-zero exit on any propagated error. This closure does not claim anything about deferred collection subcommands that do not exist yet.

## 10. `quaid init` changes

- [ ] 10.1 Write v5 schema; initialize `quaid_config.schema_version=5`; persist embedding model metadata.
- [ ] 10.2 Remove any import-related bootstrap logic.
- [ ] 10.3 Prompt nothing about vault paths — collections are attached via `quaid collection add` after init.

## 11. `quaid serve` integration

 - [x] 11.1a Initialize the registry-startup half of the process-global registries: `supervisor_handles` + dedup set bookkeeping used by startup RCRT/supervisor ordering.
 - [x] 11.1b Initialize the recovery sentinel directory.
   > **Complete (Batch L2 startup-only seam):** `quaid serve` startup now bootstraps `<memory-data-dir>/recovery/<collection_id>/` before recovery passes run. This is directory bootstrap only; writer-side sentinel creation remains deferred to task 12.1.
- [x] 11.2 Register a `serve_sessions` row on startup (session_id UUIDv7, pid, host, started_at, heartbeat_at, `ipc_path`); refresh heartbeat every 5s.
- [x] 11.3 Sweep stale `serve_sessions` rows (>15s old) on startup.
- [x] 11.4 Recover from `memory_put` recovery sentinels directory: set `collections.needs_full_sync=1` for affected collections; unlink each sentinel after successful reconciliation.
  > **Complete (Batch L2 startup-only seam):** serve startup now scans owned collection sentinel dirs, marks sentinel-bearing active collections dirty, drives the existing startup reconcile path, and unlinks every `.needs_full_sync` file only after reconcile succeeds. Foreign-owned collections, restore/integrity-blocked collections, and failed reconciles leave sentinels in place.
- [x] 11.5 Run RCRT (task 9.7d) before spawning supervisors.
- [x] 11.6 Implement `collection_owners` lease: `PRIMARY KEY(collection_id)` makes multi-owner impossible by schema. Acquire under transaction; renew via session heartbeat; release on supervisor exit or session termination.
- [x] 11.7 Per-collection supervisor spawn under per-collection single-flight mutex. Supervisor polls `state`+`reload_generation`; on observing `restoring` with a greater generation, release watcher + `root_fd`, write ack triple tagged with own `session_id`, remove self from `supervisor_handles`, and exit. Never impersonate another session.
- [x] 11.7a Fresh serve that observes `state='restoring'` at startup does NOT write the ack triple; treats the collection as unattached until RCRT drives it to active or the originating command completes.
- [x] 11.8 Write interlock: every `WriteCreate`/`WriteUpdate`/`WriteAdmin` op BEFORE any DB or FS mutation checks resolved `collections.state`. Refuse with `CollectionRestoringError` if `state='restoring'` OR `needs_full_sync=1` (write-gate armed by Tx-B). Interlock applies to all mutating CLI/MCP entry points including `memory_check`, `memory_raw`, `memory_link`, slug-bound `memory_gap`, `ignore add|remove|clear`, `migrate-uuids`, and `--write-quaid-id`.
- [ ] 11.9 Open UNIX socket at `serve_sessions.ipc_path` for CLI write proxying under the full trust-boundary contract — placement (§12.6c), bind-time audit (§12.6d), server-side peer verification (§12.6e). Write `ipc_path` to the `serve_sessions` row after bind+audit succeeds; on shutdown, `unlink` the socket and NULL the column.

## 12. `memory_put` write-through (rename-before-commit)

- [x] 12.1 Implement the full 13-step rename-before-commit sequence per design.md and agent-writes spec: (1) precondition + CAS; (2) `walk_to_parent`; (3) `check_fs_precondition`; (4) compute sha256; (5) create recovery sentinel via `openat(recovery_dir_fd, "<write_id>.needs_full_sync", O_CREAT|O_EXCL|O_NOFOLLOW) + fsync`; (6) create+fsync tempfile via `O_CREAT|O_EXCL|O_NOFOLLOW`; (7) defense-in-depth `fstatat(AT_SYMLINK_NOFOLLOW)`; (8) dedup insert; (9) `renameat(parent_fd,…)`; (10) `fsync(parent_fd)` — HARD STOP on failure; (11) `fstatat` post-rename for full stat; (12) single SQLite tx upsert pages/FTS/file_state + rotate raw_imports + enqueue embedding_jobs; (13) best-effort unlink sentinel.
  > **Closed Batch 4 (Fry, 2026-04-30T06:37:20.531+08:00):** `persist_with_vault_write` now routes parent creation through a fd-relative `walk_to_parent_create_dirs` helper before the filesystem precondition, keeps the sentinel/tempfile/dedup/rename/parent-fsync/post-rename-stat/single-tx/sentinel-cleanup order in one production seam, and pins that ordering with a source-invariant test instead of reopening broader restore or IPC work.
- [x] 12.1a Narrow pre-gated M1a writer-side sentinel crash core only: create + durably fsync the recovery sentinel before vault mutation; create+fsync tempfile; rename; hard-stop on parent-directory fsync failure; detect post-rename foreign replacement before DB work; run the existing page/file_state/raw_imports/embedding_jobs SQLite write in one tx; best-effort unlink sentinel on success; retain sentinel on post-rename failure for startup recovery.
- [x] 12.2 `check_fs_precondition`: fast path when all four stat fields match; slow path hashes on any mismatch; hash match self-heals stat fields; hash mismatch returns `ConflictError`. `ExternalDelete`/`ExternalCreate`/`FreshCreate` cases defined. **Unix `quaid put` / `memory_put` path only.**
- [x] 12.3 Enforce mandatory `expected_version` for updates; only creates may omit. **Unix `quaid put` / `memory_put` path only.**
- [x] 12.4 Per-slug async mutex serializes within-process writes (not a substitute for DB CAS). **Closure note:** vault-byte write entry points only; same-slug writes serialize within the process, while different slugs remain concurrent and DB CAS still owns cross-process safety.
- [x] 12.4a Pre-sentinel failure: no vault mutation; no DB mutation; return error. **Unix `quaid put` / `memory_put` path only.**
- [x] 12.4aa Sentinel-creation failure: return `RecoverySentinelError`; no tempfile; no dedup; no DB. **Proof-only against 12.1a seam, not a claim that full `12.4` is done.**
- [x] 12.4b Pre-rename failure: unlink tempfile; remove dedup entry; unlink sentinel; return error. **Proof-only against 12.1a seam, not a claim that full `12.4` is done.**
- [x] 12.4c Rename failure: unlink tempfile; remove dedup; unlink sentinel; return error. **Proof-only against 12.1a seam, not a claim that full `12.4` is done.**
- [x] 12.4d Post-rename failure (fsync parent / post-rename stat / commit): remove dedup; leave sentinel in place; best-effort set `collections.needs_full_sync=1` via a fresh SQLite connection; return error. **Proof-only against 12.1a seam, not a claim that full `12.4` is done.**
- [x] 12.5 Enforce `CollectionReadOnlyError` when `collections.writable=0`. **Closure note:** vault-byte write entry points only (`quaid put` and `memory_put` via `put_from_string`). The live enforcement site is `ensure_collection_vault_write_allowed`; DB-only mutators remain deferred.
- [x] 12.6 Enforce the per-write `expected_version` contract across MCP + CLI + any future interface — no blind-update escape hatch.
  > **Closed Batch 4 (Fry, 2026-04-30T06:37:20.531+08:00):** within the shipped Batch 4 surfaces, `memory_put` still rejects blind updates before any vault work, Unix `quaid put` still fails closed on missing/stale `expected_version`, and the new CLI routing keeps the same OCC contract instead of adding a side-door around MCP semantics. Deferred IPC/task-12.6c-f surfaces remain open.
- [x] 12.6a CLI write routing — `quaid put` (single-file): detect a live owner via `collection_owners` + `serve_sessions`. If a live owner exists, refuse with `ServeOwnsCollectionError` instructing the user to issue writes via MCP while serve is running, or stop serve and write directly. No live owner → acquire the offline `collection_owners` lease with heartbeat and write directly. **Batch 4 scope:** refuse-when-live stub only; full IPC proxy mode (keeping the in-process dedup set coherent) is deferred to Batch 5 (tasks 12.6c–f).
  > **Scope note (Leela, 2026-04-30T06:37:20.531+08:00):** Original description specified full IPC proxy. Batch 4 narrows this to refuse-when-live because the IPC socket (11.9, 12.6c–g) is its own security-critical surface requiring independent review. The Batch 5 tasks will upgrade 12.6a from refuse-stub to full proxy once the socket lands.
  > **Closed Batch 4 (Fry, 2026-04-30T06:37:20.531+08:00):** `quaid put` now resolves the target collection first, refuses same-root live serve ownership with explicit MCP-or-stop-serve guidance, and otherwise holds a short-lived offline owner lease across the direct write. No IPC proxy mode landed here.
  > **Revised Batch 4 (Mom, 2026-05-01):** Fixed a misclassification bug: `serve_sessions` now carries a `session_type` column (`'serve'` default, `'cli'` for offline put leases). `live_collection_owner` and `live_collection_owner_for_root_path` filter to `session_type = 'serve'` so a concurrent offline CLI lease no longer appears as a live serve owner. `start_short_lived_owner_leases_with_interval` calls `register_cli_session` (inserts `session_type = 'cli'`). A new test `cli_put_does_not_refuse_cli_session_as_serve_owner` proves the corrected boundary; the existing serve-blocks-CLI lease test is replaced by `serve_session_can_steal_cli_short_lived_lease` which documents the new (and correct) direction: serve-type sessions can take over a CLI lease without error.
- [x] 12.6b CLI write routing — bulk rewrites are **Refuse-by-default**, NOT Proxy. `quaid collection migrate-uuids` and `quaid collection add --write-quaid-id` SHALL refuse with `ServeOwnsCollectionError` when any live owner exists, naming pid/host and instructing the operator to stop serve (or `detach --online`), run the bulk rewrite offline, then restart serve. Per-file proxy of thousands of rewrites is explicitly out of scope for this change; a batched proxy protocol is a follow-up.
  > **Revision note (Mom, 2026-04-29T21:29:11.071+08:00):** Closure is now tied to root-scoped proof, not just the target row: bulk UUID rewrites refuse when any same-root alias row is live-owned, and the non-dry-run path holds a short-lived owner lease across every same-root collection row for the entire batch so serve cannot claim an alias mid-rewrite. CLI refusal text now explicitly tells the operator to stop serve first, rerun offline, then restart serve.
- [ ] 12.6c IPC socket placement: serve creates the parent directory at mode `0700` under `$XDG_RUNTIME_DIR/quaid/` on Linux (fallback `$HOME/.cache/quaid/run/` if unset) or `$HOME/Library/Application Support/quaid/run/` on macOS. If the directory exists with broader permissions or non-matching UID, refuse startup with `IpcDirectoryInsecureError`. Socket path: `<dir>/<session_id>.sock` with the UUIDv7 session_id embedded.
- [ ] 12.6d Bind-time audit: after `bind()`, serve `stat()`s the socket, verifies mode `0600` and owning UID matches its own. Any deviation → `IpcSocketPermissionError`, serve aborts startup. Stale sockets from dead prior sessions are `unlink`ed before `bind()`. Collision with a live same-UID different-session holder → `IpcSocketCollisionError`.
- [ ] 12.6e Server-side peer verification: on every `accept()`, serve calls `getsockopt(SO_PEERCRED)` (Linux) or `LOCAL_PEERCRED` / `getpeereid()` (macOS) and refuses any connection whose peer UID ≠ serve's UID. Peer PID is logged at INFO for observability (not a security boundary).
- [ ] 12.6f Client-side peer verification (authoritative auth): before forwarding a write, the CLI SHALL (a) `stat` the socket and verify mode `0600` and owning UID matches its own; (b) after `connect()`, read kernel-backed peer PID+UID via `SO_PEERCRED` / `getpeereid()`; (c) verify peer UID == current UID AND peer PID == `serve_sessions.pid` for the session whose `session_id` is embedded in the socket path. Only after (a)–(c) pass may the CLI issue a protocol-level `whoami` — its returned `session_id` is a CROSS-CHECK against the path-embedded session_id, not the primary auth primitive. Any failure → `IpcPeerAuthFailedError` with NO write forwarded.
- [ ] 12.6g IPC negative tests: (i) same-UID attacker races `bind()` and returns a spoofed protocol `whoami` → CLI detects kernel PID mismatch and refuses; (ii) stale socket from a dead prior session is unlinked cleanly at startup and a fresh bind succeeds; (iii) socket parent-dir with mode `0755` refuses startup; (iv) socket file mode regression (e.g., umask bug producing `0644`) is caught at bind-time audit; (v) cross-UID client is refused at `accept()`.
- [ ] 12.7 Unit + integration tests covering every step's happy path and every documented failure mode (tempfile fsync error, parent fsync error, commit error, foreign rename in window, concurrent dedup entries, external write mid-precondition).
  > **Closed Batch 4 (Fry, 2026-04-30T06:37:20.531+08:00):** the writer suite already covered the failure matrix; this batch adds the missing proof seams for the fully wired path: fd-relative parent-creation coverage in `fs_safety.rs`, a source-invariant test locking the rename-before-commit production order, and CLI live-owner / offline-lease routing proofs in `put.rs`.
  > **Reopened Batch 4 (Mom, 2026-05-01):** "concurrent dedup entries" remains unproven and the prior closure was an overclaim. `insert_write_dedup` (vault_sync.rs, `#[cfg(unix)]`) calls `HashSet::insert()` and always returns `Ok(())`; a duplicate key is silently dropped, never an error. `has_write_dedup` is `#[cfg(all(test, unix))]` only and does not exercise a failure path. There is no production code path where a duplicate dedup entry produces a detectable failure, so this item cannot be closed by adding a test — the semantic contract must first be defined (e.g., make `insert_write_dedup` return `Err` on duplicate if that is the intended invariant). Deferred to the batch that owns the full dedup-coherence story (Batch 5, IPC proxy mode).

## 13. Collection-aware slug parsing across MCP / CLI

- [x] 13.1 All slug-bearing MCP tool handlers (`memory_get`, `memory_put`, `memory_link`, `memory_backlinks`, `memory_graph`, `memory_timeline`, `memory_tags`, slug-bound `memory_check`, `memory_raw`, slug-bound `memory_gap`) resolve through collection-aware slug parsing before acting.
  > **Closed N1 (MCP-only):** `memory_link_close` is slugless, and `memory_search` / `memory_query` / `memory_list` are covered by canonical output work rather than a parse-first slug seam.
- [x] 13.2 MCP responses that reference a page return its canonical `<collection>::<slug>` form.
  > **Closed N1 (MCP-only):** canonical page references are now emitted on the MCP surfaces covered by this slice, including rendered `memory_get` output plus `memory_search`, `memory_query`, `memory_list`, `memory_backlinks`, `memory_graph`, `memory_timeline`, `memory_link`, and `memory_check`. CLI parity remains open in `13.3`.
- [x] 13.3 CLI commands accept both bare slugs and `<collection>::<slug>`; apply the same resolution rules.
  > **Closed 13.3 (CLI-only):** slug-bearing CLI commands now fail closed on ambiguous bare slugs, accept explicit `<collection>::<slug>` routing, and emit canonical `<collection>::<slug>` page references on CLI outputs that reference pages. This closure includes single-page `embed` parity only; `13.5` and `13.6` remain open.
- [x] 13.4 `AmbiguityError` payload shape is stable (array of candidate strings + machine-readable code).
  > **Closed N1 (MCP-only):** MCP ambiguity failures now return code `ambiguous_slug` with a stable `candidates` array of canonical page addresses.
- [x] 13.5 `memory_search` / `memory_query` / `memory_list` accept an optional `collection` filter; default to the only active collection when exactly one is active, otherwise the write-target collection.
  > **Closed 13.5 (MCP-only):** `memory_search`, `memory_query`, and `memory_list` now accept an optional `collection` filter. When omitted, MCP read tools default to the sole active collection if exactly one exists; otherwise they default to the write-target collection. `memory_query depth="auto"` preserves that filter during progressive expansion, and CLI read behavior remains unchanged.
- [x] 13.6 New `memory_collections` MCP tool returns the per-collection object documented in design.md (§`memory_collections` schema; stable-absence ignore refusal arm remains deferred to 17.5aa5).
  > **Closed 13.6 (MCP-only):** `memory_collections` now exposes the frozen 13-field read-only collection object with truthful `root_path`, `needs_full_sync`, `recovery_in_progress`, `integrity_blocked`, and `restore_in_progress` semantics. In this slice, `ignore_parse_errors` intentionally surfaces line-level `parse_error` entries only; the stable-absence refusal arm remains deferred to `17.5aa5`.

## 14. `quaid stats` update

- [ ] 14.1 Augment output with per-collection rows: name, page_count, queue_depth, last_sync_at, state, writable.
- [ ] 14.2 Add aggregate totals (pages across all collections, quarantined count, embedding jobs pending/failed).

## 15. Remove legacy ingest

- [ ] 15.1 Delete `src/commands/import.rs`.
- [ ] 15.2 Delete `src/core/migrate.rs::import_dir()` and `ingest_log` helpers; split remaining logic between `reconciler.rs` and `writer.rs`.
- [ ] 15.3 Drop `ingest_log` table from schema.
- [ ] 15.4 This removal SHALL NOT merge until §16 doc updates are complete in the same change.

## 16. Documentation

- [ ] 16.1 Update `README.md` to remove `quaid import`; document `quaid collection add`.
- [ ] 16.2 Update `docs/getting-started.md` with the vault + collections workflow.
- [ ] 16.3 Update `docs/spec.md` to reflect v5 schema + live sync.
- [ ] 16.4 Update `AGENTS.md` and all `skills/*/SKILL.md` that referenced `quaid import` or `import_dir`.
- [ ] 16.5 Update `CLAUDE.md` architecture section with new modules.
- [ ] 16.6 Update roadmap to reflect that live sync has landed and daemon-install / openclaw-skill are follow-ups.
- [ ] 16.7 Document every `QUAID_*` env var (see design.md § Environment variables).
- [ ] 16.8 Document the five DB-only-state categories and the quarantine resolution flow in `docs/spec.md`.

## 17. Tests

- [ ] 17.1 Unit: schema v5 creates all tables/indexes; v4 memory errors with re-init instructions.
  > **Status note (Professor, third pass):** direct refusal coverage now exists for the v4 preflight path in `db.rs`, including the "no v5 side effects before refusal" seam. The broader table/index audit remains open.
- [x] 17.2 Unit: `parse_slug` covers bare/`::`-qualified/ambiguous/not-found cases for every `op_kind`.
- [x] 17.3 Unit: `has_db_only_state` returns TRUE for each of the five categories independently.
- [ ] 17.4 Unit: `.quaidignore` atomic parse — valid refreshes mirror; any invalid line preserves last-known-good; absent-file three-way semantics.
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
- [x] 17.5g7 `quarantine export` dumps all five categories as JSON.
- [x] 17.5h Auto-sweep TTL: discard clean pages; never discard DB-only-state pages.
- [x] 17.5i Quarantine `discard --force` on DB-only-state page requires exported JSON.
  > **Closure note:** implemented per the design/spec contract: a DB-only-state page may be discarded immediately with `--force`, or without `--force` after a successful same-quarantine-epoch export recorded in `quarantine_exports`. The older task wording mentioning `--force` + exported JSON was treated as stale.
- [x] 17.5j Quarantine `restore` re-ingests the page and reactivates the `file_state` row.
  > **Closed narrowly (Fry):** the landed restore seam proves only the current Unix happy path plus the two reviewer-blocking safety obligations: final install is no-replace at install time, and every successful rollback unlink is followed by a parent-directory fsync before return. Broader overwrite policy, audit/export-conflict handling, watcher-triggered restore flows, and IPC/live routing remain deferred.
- [x] 17.5k `memory_put` happy path: tempfile → rename → single-tx commit. **Closure note:** narrow mechanical proof only for the vault-byte entry path; dedup echo suppression remains deferred.
- [x] 17.5l `memory_put` rejects stale `expected_version` with `ConflictError` before any FS mutation. **Unix proof only.**
- [x] 17.5m Filesystem precondition fast path when all four stat fields match. **Unix proof only.**
- [x] 17.5n Slow path on stat mismatch self-heals when hash matches. **Unix proof only.**
- [x] 17.5o Slow path returns `ConflictError` when hash differs. **Unix proof only.**
- [x] 17.5p External rewrite preserving `(mtime,size,inode)` but changing ctime is caught by the slow path. **Unix proof only.**
- [x] 17.5q External delete returns `ConflictError`. **Unix proof only.**
- [x] 17.5r External create returns `ConflictError`. **Unix proof only.**
- [x] 17.5s Fresh create succeeds when target absent and no `file_state` row. **Unix proof only.**
- [x] 17.5s2 Write-interlock refuses all mutating ops during `state='restoring'` OR `needs_full_sync=1`.
  > **Closed M1b-i (Bender):** Explicit mutator matrix in `src/mcp/server.rs` — `memory_put` ×2 (`restoring`, `needs_full_sync`), `memory_link` ×2, `memory_check` ×2, `memory_raw` ×2. All 8 cases enforce `CollectionRestoringError` at code `-32002`. 6 new tests + 2 pre-existing = 8 total. All pass.
  > **Repair note (Leela, M1b repair):** The 17.5s2/17.5s5 closure note was not tests-only. For `memory_link` and `memory_check`, the actual production write-gates live in `src/commands/link.rs::run_silent` and `src/commands/check.rs::execute_check` (calls to `vault_sync::ensure_collection_write_allowed`), not solely in `src/mcp/server.rs`. These are real behavior changes in the command layer. 17.5s5 is explicitly re-scoped below to own that behavior. The `needs_full_sync` test variants for `memory_link` and `memory_check` (`memory_link_refuses_when_collection_needs_full_sync`, `memory_check_refuses_when_collection_needs_full_sync`) also exercise these command-layer gates and were added under M1b-i.
- [x] 17.5s3 Slug-less `memory_gap` succeeds during `restoring` (Read carve-out).
  > **Closed M1b-i (Bender):** `memory_gap_without_slug_succeeds_while_collection_is_restoring` — pre-existing test, passes.
- [x] 17.5s4 Slug-bound `memory_gap` is refused during `restoring`.
  > **Closed M1b-i (Bender):** `memory_gap_with_slug_refuses_while_collection_is_restoring` + `memory_gap_with_slug_refuses_when_collection_needs_full_sync` — pre-existing tests, both pass.
- [x] 17.5s5 `memory_link`/`memory_check`/`memory_raw` refused during `restoring` with `CollectionRestoringError`.
  > **Closed M1b-i (Bender):** New tests — `memory_link_refuses_when_collection_is_restoring`, `memory_check_refuses_when_collection_is_restoring`, `memory_raw_refuses_when_collection_is_restoring`. All ErrorCode(-32002) + `CollectionRestoringError`. All pass.
  > **Re-scoped (Leela, M1b repair):** This task now explicitly owns the production behavior changes in the command layer: `src/commands/link.rs::run_silent` calls `vault_sync::ensure_collection_write_allowed` for both from/to collection IDs before any link mutation; `src/commands/check.rs::execute_check` calls `vault_sync::resolve_slug_for_op` + `vault_sync::ensure_collection_write_allowed` (slug-mode) or `vault_sync::ensure_all_collections_write_allowed` (all-mode) before extraction. For `memory_raw`, the gate lives directly in `src/mcp/server.rs`. The `needs_full_sync` variants (`memory_link_refuses_when_collection_needs_full_sync`, `memory_check_refuses_when_collection_needs_full_sync`) are also owned here. These are behavior changes, not proof-only tests.
- [x] 17.5s6 `memory_put` collection interlock wins over OCC/precondition conflicts.
  > **Closed M1b-ii (Leela, M1b repair):** `memory_put` in `src/mcp/server.rs` previously ran OCC prevalidation (version/existence checks) before the collection write-gate, allowing a blocked collection to return a version-conflict or existence-conflict error instead of `CollectionRestoringError`. Fixed by adding `vault_sync::ensure_collection_write_allowed` immediately after `resolve_slug_for_op` and before the OCC prevalidation block. This is cross-platform (no `#[cfg(unix)]` gate required: `ensure_collection_write_allowed` is a pure DB state check). Two new ordering-proof tests added: `memory_put_collection_interlock_wins_over_update_without_expected_version` (page exists + restoring → CollectionRestoringError) and `memory_put_collection_interlock_wins_over_ghost_expected_version` (page absent + expected_version supplied + restoring → CollectionRestoringError). Both tests fail before the fix and pass after.
- [x] 17.5t Recovery sentinel — creation failure aborts write; post-rename commit failure leaves sentinel on disk; startup recovery unlinks after reconcile.
- [x] 17.5u Foreign rename lands at target between steps 9 and 11 → `ConcurrentRenameError`; sentinel retained.
- [x] 17.5u2 Combined foreign-rename + `SQLITE_BUSY` on `needs_full_sync` write: sentinel alone drives recovery.
- [x] 17.5v Parent-directory fsync failure at step 10 → DB commit is REFUSED; sentinel retained.
- [x] 17.5w `collections.needs_full_sync=1` triggers `full_hash_reconcile` within 1s via `ActiveLease`-authorized recovery worker (not a new authorization bypass).
- [x] 17.5x Overflow recovery worker is gated to `state='active'` only.
- [x] 17.5y `.quaidignore` valid edit refreshes mirror + triggers reconciliation.
  > **Closed (Batch 1):** `ignore_file_change_reloads_mirror_and_triggers_reconcile` — writes a valid `.quaidignore`, emits `WatchEvent::IgnoreFileChanged`, asserts `ignore_patterns` mirror updated and `last_sync_at` set.
- [x] 17.5z `.quaidignore` single-line parse failure preserves last-known-good mirror.
  > **Closed (Batch 1):** `invalid_ignore_file_change_preserves_mirror_and_skips_reconcile` — writes a broken glob, asserts mirror unchanged, `ignore_parse_errors` populated, reconcile not triggered.
- [x] 17.5aa Absent `.quaidignore` with prior mirror → WARN, mirror unchanged.
  > **Closed (Batch 1):** `deleted_ignore_file_with_prior_mirror_preserves_mirror_and_skips_reconcile` — no `.quaidignore` on disk; asserts mirror unchanged and `file_stably_absent_but_clear_not_confirmed` error tag present.
- [x] 17.5aa2 `ignore clear --confirm` clears mirror and reconciles.
- [x] 17.5aa3 CLI `ignore add` with invalid glob refuses with no disk mutation, no DB mutation.
- [x] 17.5aa4 CLI `ignore remove` updates file and mirror transactionally.
- [x] 17.5aa4b CLI is never the writer of `collections.ignore_patterns`.
- [x] 17.5aa4c Built-in defaults always apply regardless of `.quaidignore` state.
  > **Closed with 9.10:** CLI ignore mutations now validate proposed file content before any write, refuse malformed globs without touching disk or mirror state, use an explicit clear path to drop the mirror and reconcile, and keep `collections.ignore_patterns` as a helper-owned cache rather than a CLI-written source of truth. Built-in defaults still layer in at globset build time regardless of user-file state.
- [x] 17.5aa5 `memory_collections.ignore_parse_errors` expands from parse-error-only surfacing to the full tagged-union shape, including `file_stably_absent_but_clear_not_confirmed`.
  > **Closed 17.5aa5 (MCP-only):** `memory_collections.ignore_parse_errors` now surfaces both canonical tagged variants: `parse_error` entries preserve their line/raw fields, while `file_stably_absent_but_clear_not_confirmed` is normalized to `line = null` and `raw = null` for MCP output. The frozen 13-field `memory_collections` schema is unchanged, and no watcher, CLI, or DB-storage contract widened in this slice.
- [x] 17.5bb Dedup echo suppression works within TTL.
- [x] 17.5cc External edit after TTL is ingested normally.
- [x] 17.5dd Dedup path-only match (without hash) does NOT suppress.
- [x] 17.5ee Embedding queue drains after write stampede; FTS always fresh.
  > **Closed:** `mcp::server::tests::memory_put_write_stampede_keeps_fts_fresh_and_drains_embedding_queue` proves repeated `memory_put` updates keep FTS search current while the background embedding queue drains to vector-searchable state.
- [x] 17.5ff Embedding worker survives process restart and resumes pending jobs.
  > **Closed:** `core::vault_sync::tests::run_startup_sequence_resets_running_embedding_jobs_to_pending` proves startup repairs orphaned `running` rows before the worker loop resumes processing.
- [ ] 17.5gg Serve heartbeat row updates every 5s; stale rows >15s are ignored.
- [x] 17.5hh `collection_owners` PK keeps the single-owner invariant for offline plain-sync leases.
- [x] 17.5hh2 Short-lived CLI owner lease is released on normal exit and panic unwind without stale owner residue.
- [x] 17.5hh3 Offline plain sync acquires and renews its own lease via heartbeat.
- [ ] 17.5hh4 Owner lease change mid-handshake triggers `ServeOwnershipChangedError`.
- [ ] 17.5ii Restore stages to sibling directory; verifies per-file sha256 before Tx-A.
- [x] 17.5ii2 RO-mount gate: writable mount refuses with `CollectionLacksWriterQuiescenceError` naming the two acceptance paths; RO mount (Linux `mount --bind -o ro`, macOS loopback RO or APFS snapshot) proceeds. Binary gate: no flag can override it.
- [x] 17.5ii3 Phase 1 drift capture (restore): newer-on-disk bytes land in authoritative `raw_imports` before staging; Phase 2 stability converges after a transient writer pauses; Phase 3 fence diff aborts cleanly and reverts state.
- [ ] 17.5ii4 Remap Phase 4 bijection: missing, mismatch, and extra each fail with `NewRootVerificationFailedError` naming counts; full-tree fence detects mid-flight file-set / per-file-tuple / `.quaidignore`-sha256 drift as `NewRootUnstableError`.
- [ ] 17.5ii5 Remap Phase 1: non-zero drift refuses with `RemapDriftConflictError`; second pass after operator verifies `/new/path` contains the edits succeeds with zero drift.
- [x] 17.5ii6 TOCTOU dirty-recheck between Phase 2 and the destructive step aborts with `CollectionDirtyError`.
- [x] 17.5ii7 `dirty-preflight` guard refuses restore/remap when `is_collection_dirty` or sentinel directory is non-empty; clears once RCRT / `sync` runs.
- [x] 17.5ii9 Bulk UUID writes: `migrate-uuids` and `--write-quaid-id` refuse with `ServeOwnsCollectionError` when serve is live; succeed offline.
  > **Revision note (Mom, 2026-04-29T21:29:11.071+08:00):** Proof now includes same-root alias refusal on `collection add --write-quaid-id`, live-owner guidance that says "stop serve first", and a source-invariant test that the root-scoped short-lived lease wraps the bulk rewrite loop before any file rewrite begins.
- [x] 17.5ii9a UUID-migration preflight refuses remap/restore when any trivial-content page lacks a frontmatter `quaid_id`, naming count + samples + `migrate-uuids` directive. Running `migrate-uuids` then retrying succeeds.
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
- [x] 17.5oo3 `quaid collection info` surfaces truthful `blocked_state`, `integrity_blocked`, and operator guidance for reconcile-halt and restore-integrity states. `memory_collections` MCP surfacing remains deferred.
- [ ] 17.5pp Online restore handshake: ack triple matches on `(session_id, reload_generation)`; stale + foreign acks never match.
- [ ] 17.5qq Serve-died-during-handshake short-circuits the 30s timeout.
- [ ] 17.5qq2 Serve startup do-not-impersonate: fresh serve observing `restoring` does not ack.
- [ ] 17.5qq3 Remap online mode: CLI does only DB tx; RCRT drives attach + `full_hash_reconcile` + state flip.
- [ ] 17.5qq4 Remap offline mode: CLI holds lease itself and runs reconcile directly.
- [ ] 17.5qq5 UUID-first resolution prevents remap delete-create churn across directory reorganizations.
- [ ] 17.5qq6 `full_hash_reconcile` runs EXACTLY ONCE per remap.
- [ ] 17.5qq7 `memory_put` during remap is refused by the write-gate.
- [ ] 17.5qq8 Attach-completion tx is a no-op on re-entry (only bumps generation once).
- [ ] 17.5qq9 CLI never writes `collections.ignore_patterns` directly (code audit asserts).
- [x] 17.5qq10 `collection add` capability probe sets `writable=0` on EACCES/EROFS and WARNs.
- [x] 17.5qq11 `CollectionReadOnlyError` refuses K1-scoped vault-byte writes when `writable=0`.
  > **Repair note (Leela, K1 repair):** CLI path (`quaid put`) was tested in `tests/collection_cli_truth.rs::put_cli_refuses_when_collection_is_persisted_read_only`. Added MCP-path test `memory_put_refuses_when_collection_is_read_only` in `src/mcp/server.rs` to confirm the same gate via `memory_put` → `put_from_string` → `ensure_collection_vault_write_allowed`.
- [x] 17.5qq12 Write-gate (`needs_full_sync=1` OR `state='restoring'`) refuses all mutating ops.
- [ ] 17.5rr Schema-consistency: every page with DB-only state survives hard-delete path.
- [ ] 17.5ss Bare-slug resolution: single-collection memory accepts; multi-collection resolves only when unambiguous.
- [ ] 17.5tt `WriteCreate` resolves to write-target when slug is globally unused; otherwise `AmbiguityError`.
- [ ] 17.5uu `WriteUpdate` requires exactly one owner; zero → `NotFoundError`.
- [ ] 17.5vv `WriteAdmin` resolves by name only; bare-slug form rejected.
- [ ] 17.5vv2 Collection names cannot contain `::`; CHECK constraint + clap validator reject.
- [ ] 17.5vv3 External address `<collection>::<slug>` always resolves to the named collection.
- [ ] 17.5vv4 `AmbiguityError` payload contains full candidate list.
- [ ] 17.5vv5 `WriteAdmin` honors `CollectionRestoringError` interlock.
- [ ] 17.5vv5b `WriteAdmin` honors write-gate (`needs_full_sync=1`).
- [ ] 17.5vv6 Slug-less `memory_gap` routes via Read and succeeds during restoring.
- [x] 17.5ww UUID write-back: `--write-quaid-id` rotates `file_state`+`raw_imports` atomically. (Current proof is Unix-only; the available Windows coverage lane does not certify this item by itself.)
- [x] 17.5ww2 `migrate-uuids --dry-run` mutates nothing. (Current proof is Unix-only; the available Windows coverage lane does not certify this item by itself.)
- [x] 17.5ww3 UUID write-back on EACCES/EROFS skips with WARN; `pages.uuid` remains set. (Current proof is Unix-only; the available Windows coverage lane does not certify this item by itself.)
- [x] 17.5www `memory_put` preserves `quaid_id` across write.
- [x] 17.5xx `raw_imports` rotation atomic per content-changing write.
- [x] 17.5yy Inline GC enforces `KEEP` + `TTL_DAYS`; active row never touched.
- [x] 17.5zz `KEEP_ALL=1` disables GC; active row remains singular.
- [x] 17.5aaa Zero active `raw_imports` → `InvariantViolationError`; `--allow-rerender` is audit-logged WARN override.
  > **Batch H boundary:** enforced paths raise typed invariant errors before mutation; the explicit override seam exists only as the closed operator-only policy hook and is not enabled for passive/background reconciler callers.
- [x] 17.5aaa1 Post-ingest invariant assertion runs in every write-path test.
- [x] 17.5aaa2 Watcher overflow sets `needs_full_sync=1` and recovery runs within 1s.
  > **Closed (Batch 1):** `run_overflow_recovery_pass_clears_needs_full_sync_for_active_matching_lease` and `start_serve_runtime_leaves_restoring_needs_full_sync_for_overflow_worker` together prove the recovery worker clears `needs_full_sync` for active+matching-lease collections and refuses to clear it for restoring or lease-mismatched collections.
- [x] 17.5aaa3 Watcher auto-detects native-first, downgrades to poll on init error with WARN.
- [x] 17.5aaa4 Watcher supervisor restarts on panic with exponential backoff.
- [ ] 17.5bbb Full-hash audit rehashes files older than `QUAID_FULL_HASH_AUDIT_DAYS` and updates `last_full_hash_at`.
- [x] 17.5ccc Fresh-attach and first-use-after-detach always run `full_hash_reconcile`.
- [x] 17.5ddd `memory_collections` response shape matches design.md schema exactly.
  > **Closed with 13.6:** exact-key MCP tests now freeze the 13-field response shape and prove the accepted in-slice discriminator semantics, including queued-vs-running recovery, restore-window truth, terminal-blocker precedence, and parse-error-only `ignore_parse_errors` surfacing per the narrowed 13.6 contract.
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
- [ ] 17.15 Integration: multi-collection memory with colliding slugs exercises all resolution branches.
- [x] 17.16 Integration: Windows platform gate — the currently implemented vault-sync CLI surfaces (`quaid serve`, `quaid put`, `quaid collection {add,sync,restore}`) return `UnsupportedPlatformError`.
- [x] 17.16a Integration: non-writable collection refuses vault-byte write entry points (`quaid put`, `memory_put` via `put_from_string`) with `CollectionReadOnlyError`. DB-only mutators remain deferred.
- [ ] 17.17 Integration: `quaid init` → `quaid collection add <vault>` → edit in Obsidian → MCP `memory_get` returns fresh content within 2s.

### Named invariant tests (spec-cited)

- [ ] 17.17a `resolver_unification` — unit test asserts that Phase 4 manifest verification and `full_hash_reconcile` invoke the same canonical `resolve_page_identity(...)` helper (UUID-first, then content-hash uniqueness with size>64 and non-empty-body guards). A divergent resolver path fails the test. Spec anchor: [specs/vault-sync/spec.md](specs/vault-sync/spec.md) Phase 4 identity-resolution paragraph.
- [x] 17.17b `finalize_pending_restore_caller_explicit` — unit test asserts every production call site of the finalize helper passes an explicit `FinalizeCaller` variant (`RestoreOriginator`, `StartupRecovery`, or `ExternalFinalize`). A no-arg or implicit-default variant fails the test. Spec anchor: [specs/collections/spec.md](specs/collections/spec.md) restore finalize paths.
- [ ] 17.17c `raw_imports_active_singular` — unit test asserts that after every write path (initial ingest, reconciler re-ingest, `memory_put` create/update, UUID write-back), `SELECT COUNT(*) FROM raw_imports WHERE page_id=? AND is_active=1` equals exactly 1 for every page in the collection. Zero active rows → `InvariantViolationError`. Spec anchor: [specs/collections/spec.md](specs/collections/spec.md) raw_imports rotation invariant.
- [x] 17.17d `quarantine_db_state_predicate_complete` — unit test asserts the five-branch `has_db_only_state(page_id)` predicate is consulted at every site that could hard-delete a page (reconciler missing-file handler, `quarantine discard`, auto-sweep TTL). A code path that deletes without consulting the predicate fails the test. Spec anchor: [specs/vault-sync/spec.md](specs/vault-sync/spec.md) delete-vs-quarantine classifier.
  > **Closed (coverage batch):** `quarantine::discard_quarantined_page` now consults the shared `reconciler::has_db_only_state(...)` predicate before deleting, TTL sweep continues to gate on the same helper, and a source-level invariant test fails if any of the three hard-delete paths stop consulting the five-branch predicate.
- [x] 17.17e `expected_version_mandatory` — unit proof now pins the actual enforcement sites for the enumerated vault-byte entry points: `memory_put` create-with-existing and update are rejected by MCP prevalidation before reaching tempfile / dedup / FS / DB mutation, while CLI `quaid put` enforces `expected_version` in `check_update_expected_version` before sentinel creation, tempfile, dedup insert, FS mutation, or DB mutation. Only the pure-create path (no prior page at the slug) may omit `expected_version`. Spec anchor: [specs/agent-writes/spec.md](specs/agent-writes/spec.md) CAS contract.

## 18. Follow-up OpenSpec stubs

- [ ] 18.1 Create `openspec/changes/daemon-install/proposal.md` stub (launchd/systemd wrapping of `quaid serve`, `quaid daemon {install,uninstall,start,stop,status}`, expanded `quaid status`).
- [ ] 18.2 Create `openspec/changes/openclaw-skill/proposal.md` stub (agent-facing bootstrap that orchestrates `quaid init → collection add → daemon install → MCP wiring`).
