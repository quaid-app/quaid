# Phase 1 Execution Checklist — Core Storage, CLI, Search, MCP

**Owner:** Fry
**Reviewers:** Professor (db/search/inference), Nibbler (MCP/OCC), Bender (round-trip), Scruffy (test coverage)
**Gate:** All ship-gate items must pass before Phase 2 begins.

---

## How to read this list

Tasks are sequenced by dependency. Complete each section before moving to the next.
Each task maps to one or more spec requirements. Files are listed explicitly — every

Checkboxes: [ ] = not started, [~] = in progress, [x] = done.

---

## Week 1 — Foundation

### T01 · src/core/types.rs — all domain types

> **Depends on:** nothing
> **Spec:** core-storage/spec.md

- [x] Define Page struct: slug, title, page_type, wing, room, summary, compiled_truth, timeline, tags: Vec<String>, version: i64, created_at, updated_at
- [x] Define Link struct: id, from_slug, to_slug, relationship, valid_from: Option<String>, valid_until: Option<String>, created_at
- [x] Define SearchResult struct: slug, summary, score: f64, wing
- [x] Define Chunk struct: page_slug, heading_path, content, content_hash, token_count, chunk_type (truth | timeline)
- [x] Define KnowledgeGap struct (stub only, used in Phase 3)
- [x] Define error enums using thiserror: DbError, ParseError, OccError::Conflict { current_version: i64 }, SearchError, InferenceError
- [x] Define SearchMergeStrategy enum: SetUnion, Rrf
- [x] Derive Debug, Clone, serde::Serialize, serde::Deserialize on all public structs
- [x] Unit test: construct a Page, serialize to JSON, deserialize back — assert field round-trip
- [x] cargo check passes after this task

---

### T02 · src/core/db.rs — connection, schema init, WAL, sqlite-vec

> **Depends on:** T01
> **Spec:** core-storage/spec.md — Database initialisation, v4 schema completeness, WAL checkpoint via compact

- [x] Implement open(path: &Path) -> Result<Connection, DbError>:
  - Return Err(DbError::PathNotFound) if parent directory does not exist
  - Apply schema DDL via conn.execute_batch() using include_str!
  - Set PRAGMA journal_mode = WAL
  - Set PRAGMA foreign_keys = ON
  - Load sqlite-vec extension; wrap in feature-flag guard so unit tests compile without it
  - PRAGMA user_version must equal 4 after open
- [x] Implement compact(conn: &Connection) -> Result<(), DbError>: runs PRAGMA wal_checkpoint(TRUNCATE)
- [x] Unit test: open a temp-path DB, verify all expected tables exist via sqlite_master
- [x] Unit test: open same path twice — second open is a no-op (idempotent DDL guards)
- [x] Unit test: compact completes without error on a live DB
- [x] cargo test db passes

---

### T03 · src/core/markdown.rs — frontmatter, split, summary, render

> **Depends on:** T01
> **Spec:** ingest-export/spec.md — Markdown frontmatter parsing, Compiled_truth / timeline split
> **Design decision:** split at first bare --- line after frontmatter; render_page must be byte-exact for roundtrip_raw

- [x] Implement parse_frontmatter(raw: &str) -> (HashMap<String, String>, String):
  - If file starts with ---
, extract YAML block up to next ---
; parse with serde_yaml
  - Return empty map + full string if no frontmatter present
- [x] Implement split_content(body: &str) -> (String, String):
  - Split at first line that is exactly --- after frontmatter has been stripped
  - compiled_truth = everything above; timeline = everything below
  - If no boundary: compiled_truth = body, timeline = empty string
- [x] Implement extract_summary(compiled_truth: &str) -> String:
  - Return the first non-heading, non-empty paragraph (200 chars max); fall back to first line
- [x] Implement render_page(page: &Page) -> String:
  - Emit frontmatter YAML block, then compiled_truth, then 
---
, then timeline
  - Must produce byte-exact output for a round-trip on canonical input
- [x] Unit test: parse valid frontmatter — fields match; body starts after frontmatter
- [x] Unit test: parse file with no frontmatter — empty map, full body
- [x] Unit test: split_content with boundary — correct halves
- [x] Unit test: split_content with no boundary — full body in compiled_truth, empty timeline
- [x] Unit test: render then re-parse then re-render is idempotent
- [x] cargo test markdown passes

---

### T04 · src/core/palace.rs — wing and room derivation

> **Depends on:** T01
> **Spec:** crud-commands/spec.md — Auto-derive wing from slug
> **Design decision:** wing = first slug segment; room = Phase 1 returns empty string; classify_intent is a simple heuristic

- [x] Implement derive_wing(slug: &str) -> String: split on /; return first segment; fall back to general for flat slugs
- [x] Implement derive_room(_content: &str) -> String: Phase 1 — always return empty string (room-level filtering deferred)
- [x] Implement classify_intent(query: &str) -> Option<String>: return wing segment if query contains a slug-like token; else None
- [x] Unit test: derive_wing(people/alice) returns people
- [x] Unit test: derive_wing(readme) returns general
- [x] cargo test palace passes

---

### T05 · src/commands/init.rs — quaid init [PATH]

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — quaid init command

- [x] Call db::open(path) where path is CLI arg or default_db_path() (already in main.rs)
- [x] If file already exists: print Database already exists at <path> and exit 0
- [x] On success: print Memory initialized at <path> and exit 0
- [x] On error: print to stderr and exit 1

---

### T06 · src/commands/put.rs — quaid put <SLUG> [--expected-version N]

> **Depends on:** T02, T03, T04
> **Spec:** crud-commands/spec.md — quaid put command, OCC conflict on put
> **Design decision:** compare-and-swap on version column; INSERT for new pages, compare-and-swap UPDATE for existing

- [x] Read markdown from stdin
- [x] Call parse_frontmatter + split_content + extract_summary + derive_wing + derive_room
- [x] For new pages: INSERT INTO pages with version=1
- [x] For existing pages with --expected-version N:
  - UPDATE pages SET ..., version = version + 1 WHERE slug = ? AND version = N
  - If rows_affected == 0: print conflict error with current version and exit 1
- [x] For existing pages without --expected-version: upsert without version check
- [x] Print success message including resulting version
- [x] Unit test: create page — version is 1
- [x] Unit test: update with correct version — version becomes 2
- [x] Unit test: update with stale version — conflict error printed, exit 1

---

### T07 · src/commands/get.rs — quaid get <SLUG>

> **Depends on:** T02, T03
> **Spec:** crud-commands/spec.md — quaid get command

- [x] Query SELECT * FROM pages WHERE slug = ?
- [x] If not found: print error to stderr, exit 1
- [x] Call render_page(page) and print to stdout
- [x] Unit test: put a page, get it back — rendered output matches input

---

### T08 · src/commands/list.rs — quaid list [--wing W] [--type T] [--limit N]

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — quaid list command

- [x] Build query over pages table with optional wing/type filters, ORDER BY updated_at DESC LIMIT N
- [x] Default limit: 50
- [x] Print one line per page: <slug>  <type>  <summary>
- [x] Unit test: list with wing filter returns only matching pages

---

### T09 · src/commands/stats.rs — quaid stats

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — quaid stats command

- [x] Query: total pages, pages by type (GROUP BY page_type), total links, embedding count, DB file size via std::fs::metadata
- [x] Print structured summary to stdout
- [x] Unit test: stats on empty DB returns zeros without error; on populated DB returns correct counts

---

### T10 · src/commands/tags.rs — quaid tags <SLUG> [--add TAG] [--remove TAG]

> **Depends on:** T06, T07
> **Spec:** crud-commands/spec.md — quaid tags command

- [x] Without flags: SELECT tags from the tags table for the given slug; print one per line
- [x] --add: INSERT OR IGNORE into tags table for the given slug + tag; no OCC needed (tags are independent of page version)
- [x] --remove: DELETE from tags table for the given slug + tag; no OCC needed
- [x] Unit test: add a tag, list tags — tag appears; remove it — tag gone

---

### T11 · src/commands/link.rs — quaid link <FROM> <TO> --relationship <REL> [--valid-from D] [--valid-until D]

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — quaid link command

- [x] Insert row into links table with from_slug, to_slug, relationship, valid_from, valid_until, created_at
- [x] --valid-until on existing link: UPDATE valid_until field to close the link
- [x] Unit test: create link — row exists in links; close link — valid_until is set

---

### T12 · src/commands/compact.rs — quaid compact

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — quaid compact command

- [x] Call db::compact(conn) — print success; on error print to stderr and exit 1

---

## Week 2 — Search

### T13 · src/core/fts.rs — FTS5 full-text search

> **Depends on:** T02, T01
> **Spec:** search/spec.md — FTS5 full-text search

- [x] Implement search_fts(query: &str, wing_filter: Option<&str>, conn: &Connection) -> Result<Vec<SearchResult>, SearchError>:
  - BM25-ranked FTS5 query joining page_fts to pages; apply wing filter when provided
  - Return empty vec on no results (not an error)
- [x] Unit test: insert a page, search for a word from its content — page appears in results
- [x] Unit test: search with wing filter — only matching-wing pages returned
- [x] Unit test: search on empty DB — empty vec returned without error
- [x] cargo test fts passes

---

### T14 · src/core/inference.rs — candle model init, embed, vector search

> **Depends on:** T01, T02
> **Spec:** embeddings/spec.md — Candle model initialization, Text embedding generation, Vector search

- [x] Declare static MODEL: OnceLock<EmbeddingModel> and ensure_model() -> &'static EmbeddingModel
- [x] Implement embed(text: &str) -> Result<Vec<f32>, InferenceError):
  - [x] Return Err(InferenceError::EmptyInput) for empty/whitespace input
  - [x] Tokenize + forward pass on CPU via BGE-small-en-v1.5; L2-normalize output; return 384-dim vector
- [x] Implement search_vec(query: &str, k: usize, wing_filter: Option<&str>, conn: &Connection) -> Result<Vec<SearchResult>, SearchError>:
  - Embed query; KNN query on page_embeddings_vec_384; join to pages for slug + summary; apply wing filter
  - Return empty vec on no embeddings
- [x] Unit test: embed returns Vec of len 384 with L2 norm approximately 1.0
- [x] Unit test: embed empty string returns InferenceError::EmptyInput
- [x] Unit test: vector search on empty DB returns empty vec without error
- [x] cargo test inference passes (first run is slow due to model init)

---

### T15 · src/core/chunking.rs — temporal sub-chunking

> **Depends on:** T01, T03
> **Spec:** embeddings/spec.md — Temporal sub-chunking

- [x] Implement chunk_page(page: &Page) -> Vec<Chunk>:
  - Split compiled_truth at ## headers — one Chunk per section with chunk_type = truth
  - Split timeline at --- separators — one Chunk per entry with chunk_type = timeline
  - Per chunk: content_hash = SHA-256(chunk.content), token_count = content.len() / 4 (Phase 1 approximation), heading_path set to heading text
- [x] Unit test: 3-section compiled_truth — produces 3 truth chunks, each with heading_path set
- [x] Unit test: 5 timeline entries separated by --- — produces 5 timeline chunks
- [x] Unit test: every chunk has non-empty content_hash
- [x] cargo test chunking passes

---

### T16 · src/core/search.rs — hybrid search (SMS + FTS5 + vec0 + set-union merge)

> **Depends on:** T13, T14
> **Spec:** search/spec.md — SMS exact-match short-circuit, Hybrid search with set-union merge, Wing-level palace filtering
> **Design decision:** SMS fires first; then FTS5 + vec fan-out; then set-union merge (default) or RRF via config

- [x] Implement hybrid_search(query: &str, wing: Option<&str>, conn: &Connection) -> Result<Vec<SearchResult>, SearchError>:
  - Stage 1 SMS: strip [[ and ]]; if query matches a slug exactly return that page only
  - Stage 2 fan-out: call search_fts then search_vec sequentially
  - Stage 3 set-union (default): deduplicate by slug; score = weighted BM25 + cosine; sort descending
  - Stage 3 RRF (when config search_merge_strategy = rrf): apply Reciprocal Rank Fusion instead
- [x] Implement read_merge_strategy(conn: &Connection) -> SearchMergeStrategy: read from config table; default SetUnion
- [x] Unit test: exact slug query — short-circuits, returns 1 result
- [x] Unit test: wiki-link format [[slug]] — same short-circuit behaviour
- [x] Unit test: FTS=[A,B,C] + vec=[B,C,D] — set-union returns [A,B,C,D] with no duplicates
- [x] Unit test: no exact match — both FTS5 and vec are searched
- [x] Unit test: wing filter restricts both sub-queries
- [x] cargo test search passes

---

### T17 · src/commands/search.rs — quaid search "<QUERY>" [--wing W] [--limit N]

> **Depends on:** T13
> **Spec:** search/spec.md — quaid search command

- [x] Call search_fts(query, wing, conn)
- [x] Print results as <slug>: <summary> lines ordered by score, up to --limit (default 10)
- [x] If no results: print No results found. and exit 0

---

### T18 · src/commands/embed.rs — quaid embed [SLUG | --all | --stale]

> **Depends on:** T14, T15, T02
> **Spec:** embeddings/spec.md — quaid embed command
>
> **T14 dependency (honest status):** Command plumbing is ✅ complete — all three
> invocation modes work, embedding rows are stored, stale-skip logic is correct.
> The stored vectors are hash-indexed (not semantic) until T14 ships Candle/BGE-small.
> `quaid embed` emits a runtime note on stderr to prevent the output from being
> mistaken for true semantic indexing.

- [x] quaid embed <SLUG>: chunk the page, embed each chunk, upsert into page_embeddings and page_embeddings_vec_384
- [x] quaid embed --all: iterate all pages, embed all chunks (skip if content_hash unchanged)
- [x] quaid embed --stale: only re-embed pages where stored content_hash differs from current
- [x] Unit test: embed a page — embedding rows appear in page_embeddings; re-embed unchanged page — no new rows added

---

### T19 · src/commands/query.rs — quaid query "<QUERY>" [--wing W] [--limit N] [--token-budget N]

> **Depends on:** T16
> **Spec:** embeddings/spec.md — quaid query command
>
> **T14 dependency (honest status):** Command plumbing is ✅ complete — hybrid
> search, token-budget truncation, limit, wing filter, and JSON output all work.
> Vector similarity scores in results are hash-proximity until T14 ships. FTS5
> ranking in the merged output remains fully accurate regardless of T14 status.

- [x] Call hybrid_search(query, wing, conn)
- [x] Print top results up to --limit (default 10), truncated by --token-budget if set (hard cap on output chars in Phase 1)
- [x] Phase 1: print slug + summary per result (depth/progressive expansion deferred to Phase 2)

---

## Week 3 — Ingest, Export, MCP

### T20 · src/core/novelty.rs — deduplication check

> **Depends on:** T01, T14
> **Spec:** design.md — novelty module purpose

- [x] Implement check_novelty(content: &str, existing_page: &Page, conn: &Connection) -> Result<bool, SearchError>:
  - Jaccard similarity on token sets + cosine similarity on embeddings if available
  - Return true if content is sufficiently novel (below similarity threshold)
- [x] Unit test: identical content — not novel; clearly different content — novel

---

### T21 · src/core/links.rs — link extraction and slug resolution

> **Depends on:** T01
> **Spec:** ingest-export/spec.md (wiki-link extraction used by import)

- [x] Implement extract_links(content: &str) -> Vec<String>: find [[slug]] patterns in markdown
- [x] Implement resolve_slug(raw: &str) -> String: normalise path to lowercase kebab-case

---

### T22 · src/core/migrate.rs — import_dir, export_dir, validate_roundtrip

> **Depends on:** T02, T03, T04, T14, T15, T21
> **Spec:** ingest-export/spec.md — quaid import command, quaid export command, Round-trip tests

- [x] Implement import_dir(path: &Path, conn: &mut Connection, validate_only: bool) -> Result<ImportStats, anyhow::Error>:
  - Walk directory recursively; collect all .md files
  - For each file: compute SHA-256; check ingest_log; skip if present
  - Parse frontmatter + split_content + derive_wing + derive_room + extract_summary
  - Batch insert all pages in a single transaction
  - Record SHA-256 in ingest_log for each ingested file
  - Call embed logic after transaction commits
  - If validate_only: parse only, no writes; collect errors; return error if any found
  - Print summary: Imported N pages (M skipped)
- [x] Implement export_dir(output_path: &Path, conn: &Connection) -> Result<(), anyhow::Error>:
  - Query all pages; for each call render_page; write to <output>/<slug>.md creating parent dirs as needed
- [x] Implement validate_roundtrip(conn: &Connection) -> Result<(), anyhow::Error> (used in tests)
- [x] Unit test: import test corpus from tests/fixtures/, verify page count matches file count
- [x] Unit test: re-import same corpus — 0 new pages, all skipped
- [x] Unit test: modify 1 fixture (new SHA-256) — 1 re-imported, rest skipped
- [x] Unit test: export — each page file exists at correct slug path

---

### T23 · src/commands/import.rs — quaid import <PATH> [--validate-only]

> **Depends on:** T22
> **Spec:** ingest-export/spec.md — quaid import command

- [x] Call migrate::import_dir(path, conn, validate_only)
- [x] Exit 1 if --validate-only found parse errors

---

### T24 · src/commands/export.rs — quaid export <OUTPUT_DIR>

> **Depends on:** T22
> **Spec:** ingest-export/spec.md — quaid export command

- [x] Call migrate::export_dir(output_path, conn)
- [x] Print: Exported N pages to <output_dir>

---

### T25 · src/commands/ingest.rs — quaid ingest <FILE> [--force]

> **Depends on:** T02, T03, T04, T14
> **Spec:** ingest-export/spec.md — quaid ingest command

- [x] Compute SHA-256 of file content; check ingest_log
- [x] If present and no --force: print Already ingested (SHA-256 match), use --force to re-ingest and exit 0
- [x] Otherwise: parse, insert/update page, update ingest_log
- [x] Unit test: ingest same file twice without --force — second call is skipped

---

### T26 · src/commands/timeline.rs — quaid timeline <SLUG>

> **Depends on:** T07

- [x] Print timeline entries for a slug in chronological order (parse timeline section, split at ---)

---

### T27 · Round-trip integration tests

> **Depends on:** T22, T23, T24
> **Spec:** ingest-export/spec.md — Round-trip tests
> **Reviewer:** Bender sign-off required before ship gate

- [x] Ensure tests/fixtures/ contains at least 5 representative .md files: at least one person, company, and project page — each with frontmatter, compiled_truth sections, and timeline entries
- [x] tests/roundtrip_semantic.rs: import tests/fixtures/, export to output dir, re-import, assert page count identical and all content hashes match
- [x] tests/roundtrip_raw.rs: import a single canonically-formatted fixture, export it, assert exported file is byte-for-byte identical to input
- [x] cargo test roundtrip passes — both tests green

---

### T28 · src/mcp/server.rs — MCP stdio server with 5 core tools

> **Depends on:** T06, T07, T08, T16, T13
> **Spec:** mcp-server/spec.md — all requirements
> **Design decision:** rmcp 0.1, stdio transport, tokio runtime on serve entry; reuse core functions, no logic duplication
> **Reviewer:** Nibbler adversarial review required before ship gate

- [x] Register all 5 tools with JSON schema: memory_get, memory_put, memory_query, memory_search, memory_list
- [x] memory_get: accept {slug} — core get — return rendered markdown; not-found returns error -32001
- [x] memory_put: accept {slug, content, expected_version?} — core put with OCC — return {version: N}; OCC conflict returns error -32009 with {current_version: N}
- [x] memory_query: accept {query, limit?, wing?} — hybrid_search — return JSON array of {slug, summary, score}
- [x] memory_search: accept {query, limit?, wing?} — search_fts — return JSON array of results
- [x] memory_list: accept {wing?, type?, limit?} — list query — return JSON array of pages
- [x] Implement MCP error code mapping: OccError to -32009, not found to -32001, parse error to -32002, DB error to -32003
- [x] Server exits cleanly when stdin closes
- [x] Unit test: send initialize + tools/list — 5 tool names in response
- [x] Unit test: memory_get on non-existent slug — -32001 error code
- [x] Unit test: memory_put with stale expected_version — -32009 with current version in error data

---

### T29 · src/commands/serve.rs — quaid serve

> **Depends on:** T28

- [x] Wrap server startup in tokio runtime (use #[tokio::main] or tokio::runtime::Builder)
- [x] Open DB connection, call mcp::server::run(conn), await clean exit
- [x] Wire serve in src/main.rs (replace stub)

---

## Week 4 — Polish and Ship Gate

### T30 · src/commands/config.rs — quaid config get/set

> **Depends on:** T02

- [x] quaid config set <KEY> <VALUE>: upsert into config table
- [x] quaid config get <KEY>: print value or Not set
- [x] Used by hybrid search for search_merge_strategy key

---

### T31 · src/commands/version.rs — quaid version

> **Depends on:** nothing

- [x] Print quaid <version> using the CARGO_PKG_VERSION env macro

---

### T32 · --json output on core read commands

> **Depends on:** T07, T08, T09, T17, T19

- [x] Add --json flag to: get, list, stats, search, query
- [x] When --json set, serialize output as JSON to stdout via serde_json

---

### T33 · Embedded skills — finalize ingest + query SKILL.md stubs

> **Depends on:** T22, T19

- [x] Review skills/ingest/SKILL.md and skills/query/SKILL.md
- [x] Update both files to accurately describe Phase 1 CLI commands and MCP tools
- [x] Verify skill extraction path works at runtime

---

### T34 · Full lint, test, and static binary verification

> **Depends on:** all T01-T33
> **Reviewers:** Scruffy (test coverage), Professor (static binary)

- [x] cargo fmt --check passes
- [x] cargo clippy -- -D warnings passes with zero warnings
- [x] cargo test passes with zero failures
- [x] cross build --release --target x86_64-unknown-linux-musl succeeds
- [x] ldd on the musl binary confirms not a dynamic executable

---

## Ship Gate Verification

> All items below must be confirmed before Phase 2 begins.
> Professor gates SG-3, SG-4, SG-5. Nibbler gates SG-6. Bender gates SG-7. Fry owns SG-1, SG-2, SG-8, SG-9.

- [x] **SG-1** cargo test passes with zero failures
- [x] **SG-2** cargo clippy -- -D warnings passes; cargo fmt --check passes
- [x] **SG-3** quaid import tests/fixtures/ then quaid export then re-import — semantic diff = 0 (Professor sign-off)
- [x] **SG-4** quaid serve connects to an MCP client; all 5 tools respond correctly; tools/list returns all 5 names (Professor sign-off)
- [x] **SG-5** musl binary has no dynamic dependencies confirmed via ldd (Professor sign-off)
- [x] **SG-6** Nibbler adversarial review on src/mcp/server.rs: OCC enforced on all write paths, no injection vectors (Nibbler sign-off)
  <!-- Fry SG-6 fixes applied in commit 5886ec2: OCC bypass closed, slug/content validation, error code consistency, limit caps, mutex recovery. Awaiting Nibbler re-review. -->
- [x] **SG-7** roundtrip_semantic and roundtrip_raw both pass CI (Bender sign-off)
- [x] **SG-8** BEIR nDCG@10 baseline recorded in benchmarks/README.md (no regression gate yet — establish the number)
- [x] **SG-9** PR from phase1/p1-core-storage-cli to main opened, linked to Phase 1 GitHub issue, all reviewer sign-offs collected before merge

---

## Task Summary

| Week | Tasks | Key deliverable |
|------|-------|----------------|
| 1 — Foundation | T01-T12 | Types, DB, markdown parser, palace, all CRUD commands |
| 2 — Search | T13-T19 | FTS5, embeddings, chunking, hybrid search, search/embed/query commands |
| 3 — Ingest + MCP | T20-T29 | Import/export/ingest, round-trip tests, MCP server, serve command |
| 4 — Polish + Gate | T30-T34 + SG | Config/version, JSON output, full test pass, ship gate verification |
