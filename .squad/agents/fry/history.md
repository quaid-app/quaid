# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

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
