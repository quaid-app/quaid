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

- [ ] Define Page struct: slug, title, page_type, wing, room, summary, compiled_truth, timeline, tags: Vec<String>, version: i64, created_at, updated_at
- [ ] Define Link struct: id, from_slug, to_slug, relationship, valid_from: Option<String>, valid_until: Option<String>, created_at
- [ ] Define SearchResult struct: slug, summary, score: f64, wing
- [ ] Define Chunk struct: page_slug, heading_path, content, content_hash, token_count, chunk_type (truth | timeline)
- [ ] Define KnowledgeGap struct (stub only, used in Phase 3)
- [ ] Define error enums using thiserror: DbError, ParseError, OccError::Conflict { current_version: i64 }, SearchError, InferenceError
- [ ] Define SearchMergeStrategy enum: SetUnion, Rrf
- [ ] Derive Debug, Clone, serde::Serialize, serde::Deserialize on all public structs
- [ ] Unit test: construct a Page, serialize to JSON, deserialize back — assert field round-trip
- [ ] cargo check passes after this task

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

- [ ] Implement derive_wing(slug: &str) -> String: split on /; return first segment; fall back to general for flat slugs
- [ ] Implement derive_room(_content: &str) -> String: Phase 1 — always return empty string (room-level filtering deferred)
- [ ] Implement classify_intent(query: &str) -> Option<String>: return wing segment if query contains a slug-like token; else None
- [ ] Unit test: derive_wing(people/alice) returns people
- [ ] Unit test: derive_wing(readme) returns general
- [ ] cargo test palace passes

---

### T05 · src/commands/init.rs — gbrain init [PATH]

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — gbrain init command

- [ ] Call db::open(path) where path is CLI arg or default_db_path() (already in main.rs)
- [ ] If file already exists: print Database already exists at <path> and exit 0
- [ ] On success: print Brain initialized at <path> and exit 0
- [ ] On error: print to stderr and exit 1

---

### T06 · src/commands/put.rs — gbrain put <SLUG> [--expected-version N]

> **Depends on:** T02, T03, T04
> **Spec:** crud-commands/spec.md — gbrain put command, OCC conflict on put
> **Design decision:** compare-and-swap on version column; INSERT for new pages, compare-and-swap UPDATE for existing

- [ ] Read markdown from stdin
- [ ] Call parse_frontmatter + split_content + extract_summary + derive_wing + derive_room
- [ ] For new pages: INSERT INTO pages with version=1
- [ ] For existing pages with --expected-version N:
  - UPDATE pages SET ..., version = version + 1 WHERE slug = ? AND version = N
  - If rows_affected == 0: print conflict error with current version and exit 1
- [ ] For existing pages without --expected-version: upsert without version check
- [ ] Print success message including resulting version
- [ ] Unit test: create page — version is 1
- [ ] Unit test: update with correct version — version becomes 2
- [ ] Unit test: update with stale version — conflict error printed, exit 1

---

### T07 · src/commands/get.rs — gbrain get <SLUG>

> **Depends on:** T02, T03
> **Spec:** crud-commands/spec.md — gbrain get command

- [ ] Query SELECT * FROM pages WHERE slug = ?
- [ ] If not found: print error to stderr, exit 1
- [ ] Call render_page(page) and print to stdout
- [ ] Unit test: put a page, get it back — rendered output matches input

---

### T08 · src/commands/list.rs — gbrain list [--wing W] [--type T] [--limit N]

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — gbrain list command

- [ ] Build query over pages table with optional wing/type filters, ORDER BY updated_at DESC LIMIT N
- [ ] Default limit: 50
- [ ] Print one line per page: <slug>  <type>  <summary>
- [ ] Unit test: list with wing filter returns only matching pages

---

### T09 · src/commands/stats.rs — gbrain stats

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — gbrain stats command

- [ ] Query: total pages, pages by type (GROUP BY page_type), total links, embedding count, DB file size via std::fs::metadata
- [ ] Print structured summary to stdout
- [ ] Unit test: stats on empty DB returns zeros without error; on populated DB returns correct counts

---

### T10 · src/commands/tags.rs — gbrain tags <SLUG> [--add TAG] [--remove TAG]

> **Depends on:** T06, T07
> **Spec:** crud-commands/spec.md — gbrain tags command

- [ ] Without flags: print current tags (one per line) from pages.tags JSON array
- [ ] --add: read current page, append tag to tags list, re-put with OCC using current version
- [ ] --remove: read current page, drop tag from list, re-put with OCC using current version
- [ ] Unit test: add a tag, list tags — tag appears; remove it — tag gone

---

### T11 · src/commands/link.rs — gbrain link <FROM> <TO> --relationship <REL> [--valid-from D] [--valid-until D]

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — gbrain link command

- [ ] Insert row into links table with from_slug, to_slug, relationship, valid_from, valid_until, created_at
- [ ] --valid-until on existing link: UPDATE valid_until field to close the link
- [ ] Unit test: create link — row exists in links; close link — valid_until is set

---

### T12 · src/commands/compact.rs — gbrain compact

> **Depends on:** T02
> **Spec:** crud-commands/spec.md — gbrain compact command

- [ ] Call db::compact(conn) — print success; on error print to stderr and exit 1

---

## Week 2 — Search

### T13 · src/core/fts.rs — FTS5 full-text search

> **Depends on:** T02, T01
> **Spec:** search/spec.md — FTS5 full-text search

- [ ] Implement search_fts(query: &str, wing_filter: Option<&str>, conn: &Connection) -> Result<Vec<SearchResult>, SearchError>:
  - BM25-ranked FTS5 query joining page_fts to pages; apply wing filter when provided
  - Return empty vec on no results (not an error)
- [ ] Unit test: insert a page, search for a word from its content — page appears in results
- [ ] Unit test: search with wing filter — only matching-wing pages returned
- [ ] Unit test: search on empty DB — empty vec returned without error
- [ ] cargo test fts passes

---

### T14 · src/core/inference.rs — candle model init, embed, vector search

> **Depends on:** T01, T02
> **Spec:** embeddings/spec.md — Candle model initialization, Text embedding generation, Vector search

- [ ] Declare static MODEL: OnceLock<EmbeddingModel> and ensure_model() -> &'static EmbeddingModel
- [ ] Implement embed(text: &str) -> Result<Vec<f32>, InferenceError>:
  - Return Err(InferenceError::EmptyInput) for empty/whitespace input
  - Tokenize + forward pass on CPU via BGE-small-en-v1.5; L2-normalize output; return 384-dim vector
- [ ] Implement search_vec(query: &str, k: usize, wing_filter: Option<&str>, conn: &Connection) -> Result<Vec<SearchResult>, SearchError>:
  - Embed query; KNN query on page_embeddings_vec_384; join to pages for slug + summary; apply wing filter
  - Return empty vec on no embeddings
- [ ] Unit test: embed returns Vec of len 384 with L2 norm approximately 1.0
- [ ] Unit test: embed empty string returns InferenceError::EmptyInput
- [ ] Unit test: vector search on empty DB returns empty vec without error
- [ ] cargo test inference passes (first run is slow due to model init)

---

### T15 · src/core/chunking.rs — temporal sub-chunking

> **Depends on:** T01, T03
> **Spec:** embeddings/spec.md — Temporal sub-chunking

- [ ] Implement chunk_page(page: &Page) -> Vec<Chunk>:
  - Split compiled_truth at ## headers — one Chunk per section with chunk_type = truth
  - Split timeline at --- separators — one Chunk per entry with chunk_type = timeline
  - Per chunk: content_hash = SHA-256(chunk.content), token_count = content.len() / 4 (Phase 1 approximation), heading_path set to heading text
- [ ] Unit test: 3-section compiled_truth — produces 3 truth chunks, each with heading_path set
- [ ] Unit test: 5 timeline entries separated by --- — produces 5 timeline chunks
- [ ] Unit test: every chunk has non-empty content_hash
- [ ] cargo test chunking passes

---

### T16 · src/core/search.rs — hybrid search (SMS + FTS5 + vec0 + set-union merge)

> **Depends on:** T13, T14
> **Spec:** search/spec.md — SMS exact-match short-circuit, Hybrid search with set-union merge, Wing-level palace filtering
> **Design decision:** SMS fires first; then FTS5 + vec fan-out; then set-union merge (default) or RRF via config

- [ ] Implement hybrid_search(query: &str, wing: Option<&str>, conn: &Connection) -> Result<Vec<SearchResult>, SearchError>:
  - Stage 1 SMS: strip [[ and ]]; if query matches a slug exactly return that page only
  - Stage 2 fan-out: call search_fts then search_vec sequentially
  - Stage 3 set-union (default): deduplicate by slug; score = weighted BM25 + cosine; sort descending
  - Stage 3 RRF (when config search_merge_strategy = rrf): apply Reciprocal Rank Fusion instead
- [ ] Implement read_merge_strategy(conn: &Connection) -> SearchMergeStrategy: read from config table; default SetUnion
- [ ] Unit test: exact slug query — short-circuits, returns 1 result
- [ ] Unit test: wiki-link format [[slug]] — same short-circuit behaviour
- [ ] Unit test: FTS=[A,B,C] + vec=[B,C,D] — set-union returns [A,B,C,D] with no duplicates
- [ ] Unit test: no exact match — both FTS5 and vec are searched
- [ ] Unit test: wing filter restricts both sub-queries
- [ ] cargo test search passes

---

### T17 · src/commands/search.rs — gbrain search "<QUERY>" [--wing W] [--limit N]

> **Depends on:** T13
> **Spec:** search/spec.md — gbrain search command

- [ ] Call search_fts(query, wing, conn)
- [ ] Print results as <slug>: <summary> lines ordered by score, up to --limit (default 10)
- [ ] If no results: print No results found. and exit 0

---

### T18 · src/commands/embed.rs — gbrain embed [SLUG | --all | --stale]

> **Depends on:** T14, T15, T02
> **Spec:** embeddings/spec.md — gbrain embed command

- [ ] gbrain embed <SLUG>: chunk the page, embed each chunk, upsert into page_embeddings and page_embeddings_vec_384
- [ ] gbrain embed --all: iterate all pages, embed all chunks (skip if content_hash unchanged)
- [ ] gbrain embed --stale: only re-embed pages where stored content_hash differs from current
- [ ] Unit test: embed a page — embedding rows appear in page_embeddings; re-embed unchanged page — no new rows added

---

### T19 · src/commands/query.rs — gbrain query "<QUERY>" [--wing W] [--limit N] [--token-budget N]

> **Depends on:** T16
> **Spec:** embeddings/spec.md — gbrain query command

- [ ] Call hybrid_search(query, wing, conn)
- [ ] Print top results up to --limit (default 10), truncated by --token-budget if set (hard cap on output chars in Phase 1)
- [ ] Phase 1: print slug + summary per result (depth/progressive expansion deferred to Phase 2)

---

## Week 3 — Ingest, Export, MCP

### T20 · src/core/novelty.rs — deduplication check

> **Depends on:** T01, T14
> **Spec:** design.md — novelty module purpose

- [ ] Implement check_novelty(content: &str, existing_page: &Page, conn: &Connection) -> Result<bool, SearchError>:
  - Jaccard similarity on token sets + cosine similarity on embeddings if available
  - Return true if content is sufficiently novel (below similarity threshold)
- [ ] Unit test: identical content — not novel; clearly different content — novel

---

### T21 · src/core/links.rs — link extraction and slug resolution

> **Depends on:** T01
> **Spec:** ingest-export/spec.md (wiki-link extraction used by import)

- [ ] Implement extract_links(content: &str) -> Vec<String>: find [[slug]] patterns in markdown
- [ ] Implement resolve_slug(raw: &str) -> String: normalise path to lowercase kebab-case

---

### T22 · src/core/migrate.rs — import_dir, export_dir, validate_roundtrip

> **Depends on:** T02, T03, T04, T14, T15, T21
> **Spec:** ingest-export/spec.md — gbrain import command, gbrain export command, Round-trip tests

- [ ] Implement import_dir(path: &Path, conn: &mut Connection, validate_only: bool) -> Result<ImportStats, anyhow::Error>:
  - Walk directory recursively; collect all .md files
  - For each file: compute SHA-256; check ingest_log; skip if present
  - Parse frontmatter + split_content + derive_wing + derive_room + extract_summary
  - Batch insert all pages in a single transaction
  - Record SHA-256 in ingest_log for each ingested file
  - Call embed logic after transaction commits
  - If validate_only: parse only, no writes; collect errors; return error if any found
  - Print summary: Imported N pages (M skipped)
- [ ] Implement export_dir(output_path: &Path, conn: &Connection) -> Result<(), anyhow::Error>:
  - Query all pages; for each call render_page; write to <output>/<slug>.md creating parent dirs as needed
- [ ] Implement validate_roundtrip(conn: &Connection) -> Result<(), anyhow::Error> (used in tests)
- [ ] Unit test: import test corpus from tests/fixtures/, verify page count matches file count
- [ ] Unit test: re-import same corpus — 0 new pages, all skipped
- [ ] Unit test: modify 1 fixture (new SHA-256) — 1 re-imported, rest skipped
- [ ] Unit test: export — each page file exists at correct slug path

---

### T23 · src/commands/import.rs — gbrain import <PATH> [--validate-only]

> **Depends on:** T22
> **Spec:** ingest-export/spec.md — gbrain import command

- [ ] Call migrate::import_dir(path, conn, validate_only)
- [ ] Exit 1 if --validate-only found parse errors

---

### T24 · src/commands/export.rs — gbrain export <OUTPUT_DIR>

> **Depends on:** T22
> **Spec:** ingest-export/spec.md — gbrain export command

- [ ] Call migrate::export_dir(output_path, conn)
- [ ] Print: Exported N pages to <output_dir>

---

### T25 · src/commands/ingest.rs — gbrain ingest <FILE> [--force]

> **Depends on:** T02, T03, T04, T14
> **Spec:** ingest-export/spec.md — gbrain ingest command

- [ ] Compute SHA-256 of file content; check ingest_log
- [ ] If present and no --force: print Already ingested (SHA-256 match), use --force to re-ingest and exit 0
- [ ] Otherwise: parse, insert/update page, update ingest_log
- [ ] Unit test: ingest same file twice without --force — second call is skipped

---

### T26 · src/commands/timeline.rs — gbrain timeline <SLUG>

> **Depends on:** T07

- [ ] Print timeline entries for a slug in chronological order (parse timeline section, split at ---)

---

### T27 · Round-trip integration tests

> **Depends on:** T22, T23, T24
> **Spec:** ingest-export/spec.md — Round-trip tests
> **Reviewer:** Bender sign-off required before ship gate

- [ ] Ensure tests/fixtures/ contains at least 5 representative .md files: at least one person, company, and project page — each with frontmatter, compiled_truth sections, and timeline entries
- [ ] tests/roundtrip_semantic.rs: import tests/fixtures/, export to output dir, re-import, assert page count identical and all content hashes match
- [ ] tests/roundtrip_raw.rs: import a single canonically-formatted fixture, export it, assert exported file is byte-for-byte identical to input
- [ ] cargo test roundtrip passes — both tests green

---

### T28 · src/mcp/server.rs — MCP stdio server with 5 core tools

> **Depends on:** T06, T07, T08, T16, T13
> **Spec:** mcp-server/spec.md — all requirements
> **Design decision:** rmcp 0.1, stdio transport, tokio runtime on serve entry; reuse core functions, no logic duplication
> **Reviewer:** Nibbler adversarial review required before ship gate

- [ ] Register all 5 tools with JSON schema: brain_get, brain_put, brain_query, brain_search, brain_list
- [ ] brain_get: accept {slug} — core get — return rendered markdown; not-found returns error -32001
- [ ] brain_put: accept {slug, content, expected_version?} — core put with OCC — return {version: N}; OCC conflict returns error -32009 with {current_version: N}
- [ ] brain_query: accept {query, limit?, wing?} — hybrid_search — return JSON array of {slug, summary, score}
- [ ] brain_search: accept {query, limit?, wing?} — search_fts — return JSON array of results
- [ ] brain_list: accept {wing?, type?, limit?} — list query — return JSON array of pages
- [ ] Implement MCP error code mapping: OccError to -32009, not found to -32001, parse error to -32002, DB error to -32003
- [ ] Server exits cleanly when stdin closes
- [ ] Unit test: send initialize + tools/list — 5 tool names in response
- [ ] Unit test: brain_get on non-existent slug — -32001 error code
- [ ] Unit test: brain_put with stale expected_version — -32009 with current version in error data

---

### T29 · src/commands/serve.rs — gbrain serve

> **Depends on:** T28

- [ ] Wrap server startup in tokio runtime (use #[tokio::main] or tokio::runtime::Builder)
- [ ] Open DB connection, call mcp::server::run(conn), await clean exit
- [ ] Wire serve in src/main.rs (replace stub)

---

## Week 4 — Polish and Ship Gate

### T30 · src/commands/config.rs — gbrain config get/set

> **Depends on:** T02

- [ ] gbrain config set <KEY> <VALUE>: upsert into config table
- [ ] gbrain config get <KEY>: print value or Not set
- [ ] Used by hybrid search for search_merge_strategy key

---

### T31 · src/commands/version.rs — gbrain version

> **Depends on:** nothing

- [ ] Print gbrain <version> using the CARGO_PKG_VERSION env macro

---

### T32 · --json output on core read commands

> **Depends on:** T07, T08, T09, T17, T19

- [ ] Add --json flag to: get, list, stats, search, query
- [ ] When --json set, serialize output as JSON to stdout via serde_json

---

### T33 · Embedded skills — finalize ingest + query SKILL.md stubs

> **Depends on:** T22, T19

- [ ] Review skills/ingest/SKILL.md and skills/query/SKILL.md
- [ ] Update both files to accurately describe Phase 1 CLI commands and MCP tools
- [ ] Verify skill extraction path works at runtime

---

### T34 · Full lint, test, and static binary verification

> **Depends on:** all T01-T33
> **Reviewers:** Scruffy (test coverage), Professor (static binary)

- [ ] cargo fmt --check passes
- [ ] cargo clippy -- -D warnings passes with zero warnings
- [ ] cargo test passes with zero failures
- [ ] cross build --release --target x86_64-unknown-linux-musl succeeds
- [ ] ldd on the musl binary confirms not a dynamic executable

---

## Ship Gate Verification

> All items below must be confirmed before Phase 2 begins.
> Professor gates SG-3, SG-4, SG-5. Nibbler gates SG-6. Bender gates SG-7. Fry owns SG-1, SG-2, SG-8, SG-9.

- [ ] **SG-1** cargo test passes with zero failures
- [ ] **SG-2** cargo clippy -- -D warnings passes; cargo fmt --check passes
- [ ] **SG-3** gbrain import tests/fixtures/ then gbrain export then re-import — semantic diff = 0 (Professor sign-off)
- [ ] **SG-4** gbrain serve connects to an MCP client; all 5 tools respond correctly; tools/list returns all 5 names (Professor sign-off)
- [ ] **SG-5** musl binary has no dynamic dependencies confirmed via ldd (Professor sign-off)
- [ ] **SG-6** Nibbler adversarial review on src/mcp/server.rs: OCC enforced on all write paths, no injection vectors (Nibbler sign-off)
- [ ] **SG-7** roundtrip_semantic and roundtrip_raw both pass CI (Bender sign-off)
- [ ] **SG-8** BEIR nDCG@10 baseline recorded in benchmarks/README.md (no regression gate yet — establish the number)
- [ ] **SG-9** PR from phase1/p1-core-storage-cli to main opened, linked to Phase 1 GitHub issue, all reviewer sign-offs collected before merge

---

## Task Summary

| Week | Tasks | Key deliverable |
|------|-------|----------------|
| 1 — Foundation | T01-T12 | Types, DB, markdown parser, palace, all CRUD commands |
| 2 — Search | T13-T19 | FTS5, embeddings, chunking, hybrid search, search/embed/query commands |
| 3 — Ingest + MCP | T20-T29 | Import/export/ingest, round-trip tests, MCP server, serve command |
| 4 — Polish + Gate | T30-T34 + SG | Config/version, JSON output, full test pass, ship gate verification |
