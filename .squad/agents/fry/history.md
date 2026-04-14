# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- The core implementation target is a Rust CLI plus MCP server.
- The system is intentionally local-first and zero-network for embeddings.
- Every meaningful implementation starts with an OpenSpec proposal.
- Always run `cargo fmt --all` before committing Rust code — CI enforces `cargo fmt --check` as the first gate and will skip all subsequent steps (clippy, check, test) if formatting fails.
- Local Windows dev environment lacks MSVC SDK libs (`msvcrt.lib`), so clippy/build/test cannot run locally. Use CI (Linux) for full validation. Only `cargo fmt` works locally.
- CI runs `cargo clippy -- -D warnings` which treats all warnings as errors. Stub functions with `todo!()` bodies must prefix all params with `_` to avoid unused variable errors.
- `version.rs` was removed — it was dead code never referenced from `mod.rs` or `main.rs`.
- PR #9 review (2026-04-14): Copilot automated reviewer caught 9 issues. 7 applied in Sprint 0 scope (CLI contract alignment, schema CHECK, docs fixes, Cargo.lock, hygiene). CI clippy `-D warnings` left as-is since stubs already fixed. Repo hygiene pass removed `gh_diagnostic.py`, one-time session artifacts.
- CLI contract must match `docs/spec.md` exactly — the spec defines the scaffold's surface. `default_db_path()` must resolve to `./brain.db`, not `$HOME/brain.db`.
- `init` and `version` commands don't require a database connection; dispatch them before `db::open()` in main.
- Reviewed and proposed adoption of `rust-best-practices` skill (Apollo GraphQL handbook, 9 chapters) at `.agents/skills/rust-best-practices/`. Decision note at `.squad/decisions/inbox/fry-rust-skill-adoption.md`. Key caveats: `#[expect]` needs MSRV ≥1.81, `rustfmt` import grouping needs nightly, snapshot testing (`insta`) deferred to Phase 1 test work.
- Error handling split already matches skill guidance: `thiserror` for `src/core/`, `anyhow` for `src/commands/` and `main.rs`.

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
- Cross-agent history updates applied (Fry, Bender, Scruffy histories appended with session context).
- Ready for git commit.

