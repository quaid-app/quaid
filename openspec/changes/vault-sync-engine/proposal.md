# Vault Sync Engine

> **Pre-release software, zero users, no migration path required.** Quaid has not shipped a tagged public release; `v0.9.x` labels in the README are developer-preview build tags. There are zero external `memory.db` files. A v4→v5 migrator is out of scope. The end-state of this change removes `quaid import`, but the foundation slice keeps `quaid import` + `ingest_log` as temporary compatibility shims until the reconciler/watcher path replaces them.

## Why

Quaid currently requires users to explicitly run `quaid import <path>` to ingest markdown. There is no live sync between a vault and `memory.db` — edits made in Obsidian don't reach memory until the user remembers to re-import. That friction is why power users still reach for QMD even when Quaid's search quality is better.

With zero current users, we can redesign the ingest model cleanly. The fix: make the vault the source of truth and memory a live index of it. `quaid serve` — already required for MCP — hosts a per-collection file watcher, reacts to filesystem events, and keeps `memory.db` in sync continuously.

This change also introduces **collections** — named groupings with their own root path and ignore patterns — so one memory can span multiple vaults (`work`, `memory`, `knowledge`).

## What Changes

- **Collections.** Every page belongs to a collection. One memory hosts many collections; each has its own root path, ignore patterns, and optional write-target flag.
- **Live filesystem sync.** `quaid serve` watches every attached collection via the `notify` crate. Events flow through a debounce → stat-diff → two-tier index pipeline.
- **Split authority model.** Vault is authoritative for page content and everything derivable from page markdown (FTS, embeddings, wiki-link rows, heuristic assertions, timeline entries, frontmatter-derived tags). `memory.db` is authoritative for programmatic relationships and operational state that cannot be reconstructed from markdown (typed/temporal links, supersession-chain assertions, `raw_data`, `knowledge_gaps`, `contradictions`). Full backup = vault + `memory.db` together.
- **Durable page identity via `quaid_id` frontmatter UUID (opt-in write-back).** Every page has a UUIDv7 in `pages.uuid`. The UUID MAY appear in frontmatter as `quaid_id`, but persistence to the file is OPT-IN. Default attach and reconciliation are read-only with respect to user bytes. Write-back happens only via `quaid collection add --write-quaid-id`, `quaid collection migrate-uuids`, or `memory_put`. Rename detection priority: (1) UUID match; (2) native rename events; (3) content-hash uniqueness fallback; (4) quarantine + fresh create.
- **Rename detection with safety guards.** Native events honored directly. Hash-based inference used only when both sides are uniquely hashed AND content is non-trivial (≥64 bytes, non-empty body after frontmatter). Ambiguous refusals **quarantine** the old page.
- **Quarantine lifecycle with DB-only-state protection.** Pages are quarantined (not hard-deleted) when either (a) rename is ambiguous, or (b) the page carries any of five DB-only-state categories: programmatic `links` (`source_kind='programmatic'`), non-import `assertions`, `raw_data`, `contradictions`, or `knowledge_gaps`. Auto-sweep TTL (default 30 days) auto-discards ONLY pages where all five categories are empty. `quaid collection quarantine {list,restore,discard,export,audit}` lets users resolve explicitly.
- **`<collection>::<slug>` external addressing with ambiguity protection.** Full form uses `::` (e.g., `work::notes/meeting`). Bare slugs resolve only when unambiguous; otherwise `AmbiguityError` names candidates. Collection names cannot contain `::`.
- **Two-tier indexing.** FTS + metadata commit synchronously (~2s visibility). Embeddings are deferred via a persistent `embedding_jobs` queue drained by a bounded-concurrency worker.
- **`memory_put` rename-before-commit write sequence.** Tempfile → fsync → recovery sentinel → atomic rename → fsync parent → stat post-rename → single SQLite tx upserts pages/FTS/file_state, rotates `raw_imports`, enqueues embeddings. SQLite commit is LAST. If the process crashes between rename and commit, the vault holds new bytes, the DB holds pre-call state, and the reconciler re-ingests on next startup. DB state never leads disk state.
- **Filesystem precondition with ctime-aware hash-on-mismatch.** Fast path: all four stat fields `(mtime_ns, ctime_ns, size_bytes, inode)` match → proceed. Slow path: mismatch triggers streaming sha256 compared against stored hash. Hash match self-heals stat fields; mismatch returns `ConflictError`. Together with `expected_version` this covers MCP-vs-MCP and MCP-vs-external races.
- **`expected_version` mandatory for updates across every write interface.** Only the create path may omit it.
- **Cold-start reconciliation with ctime/inode invalidation + full-hash-on-remap + periodic audit.** Steady-state compares four stat fields; any mismatch triggers re-hash. `sync --remap-root`, `collection restore`, and first-use-after-detach force a full-hash walk. Background audit (default 7-day interval, `QUAID_FULL_HASH_AUDIT_DAYS`) catches residual drift.
- **Watcher overflow never drops events.** On bounded-channel overflow, `collections.needs_full_sync=1`, WARN log, and a recovery task runs full-hash reconciliation within ~1s. `memory_collections` and `memory_stats` surface the flag.
- **Explicit provenance columns for quarantine safety.** `links.source_kind` (`wiki_link` | `programmatic`, default `programmatic`) and existing `assertions.asserted_by` discriminate re-derivable markdown state from DB-only state. The quarantine predicate has five branches (programmatic link, non-import assertion, `raw_data`, `contradictions`, `knowledge_gaps`). Default `programmatic` is a fail-open preservation bias. v5 also adds `knowledge_gaps.page_id INTEGER NULL REFERENCES pages(id) ON DELETE CASCADE`, and `contradictions.other_page_id` becomes `ON DELETE CASCADE`.
- **Symlink-safe root boundary via fd-relative path walk (macOS + Linux only).** Every filesystem operation bounds itself inside the collection root via `openat`/`fstatat`/`renameat` with `O_NOFOLLOW`. If the root itself is a symlink, attach refuses. Walks skip symlinks with WARN. Windows is out of scope for vault-sync commands, which refuse with an unsupported-platform error.
- **Portability and atomic byte-exact restore.** `quaid collection restore` operates only against an absent/empty target. For every page, bytes are written from the page's active `raw_imports.raw_bytes` row — byte-exact. Two-phase commit (Tx-A sets `pending_root_path`; atomic `rename()`; Tx-B flips `root_path`). On Tx-B failure, recovery runs via `finalize_pending_restore()` invoked by the originating command (while alive), by the Restoring-Collection Retry Task at startup, or by `quaid collection sync <name> --finalize-pending`.
- **Active `raw_imports` rotation — strict invariant.** Every content-changing write rotates `raw_imports` in the same SQLite tx. Exactly one active row per page at all times. Restore has a single source of bytes; if zero active rows, abort with `InvariantViolationError` (undocumented `--allow-rerender` for last-resort operator recovery).
- **Bounded retention for inactive `raw_imports`.** Per-page cap `QUAID_RAW_IMPORTS_KEEP` (default 10) AND age threshold `QUAID_RAW_IMPORTS_TTL_DAYS` (default 90). Inline GC per rotation plus daily sweep. `QUAID_RAW_IMPORTS_KEEP_ALL=1` disables retention.
- **`.quaidignore` is authoritative; `collections.ignore_patterns` is a cached mirror.** On-disk file is truth; DB column is populated on every successful atomic parse. Sync is one-way (file → DB), transactional (all-or-nothing), mtime-free. CLI `ignore add|remove` writes the file; watcher refreshes the mirror.
- **Live `.quaidignore` reload — atomic parse, last-known-good on failure.** Every non-comment line validated via `globset::Glob::new` BEFORE any effect. Fully-valid parse → refresh mirror + reconcile. Any failing line → mirror UNCHANGED, `ignore_parse_errors` recorded, no reconciliation. Absent file with no prior mirror = empty patterns (defaults only), WARN logged.
- **Live-serve coordination for restore/remap (lease-based ack).** `serve_sessions` heartbeat table (5s refresh, 15s liveness). Restore/remap without `--online` refuses with `ServeOwnsCollectionError`. `--online` runs a lease-based polled handshake keyed on `(session_id, reload_generation)` — timestamp-only ack is insufficient. Root_fd lifetime is scoped to the collection session, not the serve process.
- **End-state: `quaid import` removed.** Replaced by `quaid collection add` / `sync`, but the current foundation slice intentionally keeps `quaid import` and `ingest_log` as temporary compatibility shims until §15 lands.
- **BREAKING:** v5 schema. See `design.md § Schema`. Existing `memory.db` files require re-init.

## Capabilities

### New

- `collections`: Named groupings with their own root, ignore patterns, and lifecycle commands (add/list/info/sync/remove/restore).
- `vault-sync`: Live filesystem watching, debounced event pipeline, two-tier indexing, cold-start reconciliation, rename detection.
- `agent-writes`: `memory_put` rename-before-commit write semantics with self-write dedup and sentinel-based recovery.

### Modified

- None as standalone specs. `quaid init`, `quaid stats`, `memory_get`, `memory_put`, `memory_search`, `memory_query`, `memory_list`, `memory_link` all gain collection-aware slug parsing — documented in the new specs.

## Impact

- `src/schema.sql` — v5 foundation schema: new tables (`collections`, `file_state`, `embedding_jobs`, later `serve_sessions`/`collection_owners`), modified `pages`, and a temporary retained `ingest_log` compatibility shim until §15 removes it.
- `src/core/` — new foundation module: `collections.rs`; later slices add `watcher.rs`, `reconciler.rs`, `embedding_worker.rs`, `dedup.rs`, `fs_safety.rs`. `migrate.rs` remains temporarily; `import_dir` is removed only when reconciler lands.
- `src/commands/` — future: `collection.rs` (add/list/info/sync/remove/restore/quarantine/ignore). `import.rs` remains temporarily in the foundation slice and is removed only in §15 with the doc updates in §16.
- `src/mcp/server.rs` — collection-aware slug parsing; new `memory_collections` tool.
- `Cargo.toml` — new deps: `notify`, `ignore`, `globset`, `rustix`.
- `tests/` — collection lifecycle, reconciliation, watcher, write-through, round-trip restore.
- `docs/` — update spec, getting-started, roadmap.
- Follow-up OpenSpec changes (stubs only): `daemon-install`, `openclaw-skill`.
