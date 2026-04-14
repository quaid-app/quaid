# Squad Decisions

## Active Decisions

### 2026-04-13: Core intake sources
**By:** macro88 (via Squad)
**What:** Use `docs\spec.md` as the primary product spec, GitHub issues as work intake, and OpenSpec in `openspec\` for structured change proposals and spec evolution.
**Why:** GigaBrain already has a long-form product spec, issue-driven execution, and an initialized OpenSpec workspace. Keeping all three active gives the team a stable source of truth plus a disciplined path for changes.

### 2026-04-13: OpenSpec proposal required before meaningful changes
**By:** macro88 (via Squad)
**What:** Every meaningful code, docs, docs-site, benchmark, or testing change must begin with an OpenSpec change proposal that follows the local instructions in `openspec\`. This proposal step is required in addition to Scribe's logging and decision-merging work.
**Why:** The team needs an explicit design record before implementation, not only an after-the-fact memory trail. This keeps change intent, scope, and review visible before work starts.

### 2026-04-13: Initial squad cast and model policy
**By:** macro88 (via Squad)
**What:** The squad uses a Futurama-inspired cast. Fry and Bender prefer `claude-opus-4.6`; Amy, Hermes, Zapp, and Leela prefer `claude-sonnet-4.6`; Professor, Nibbler, and Scruffy prefer `gpt-5.4`. Kif and Mom are reserved for benchmark and edge-case work with a requested target of `Gemini 3.1 Pro` when that model is available on the active surface.
**Why:** The team is intentionally specialized around implementation, review, documentation, coverage, and performance. Model preferences reflect that specialization while keeping the unavailable Gemini request visible for future surfaces.

### 2026-04-13: Sprint 0 phases, structure, and work sequencing

**By:** Leela

**What:**
GigaBrain is organized into four sequential phases. Phase gates are enforced — no phase begins until the prior phase ships:

| Phase | Name | Gate |
|-------|------|------|
| Sprint 0 | Repository Scaffold | `cargo check` passes; CI triggers on PR; all directories from spec exist |
| Phase 1 | Core Storage, CLI, Search, MCP | Round-trip tests pass; MCP connects; static binary verified |
| Phase 2 | Intelligence Layer | Phase 1 gate passed; graph + OCC + contradiction detection complete |
| Phase 3 | Polish, Benchmarks, Release | All offline CI gates pass; all 8 skills functional; GitHub Releases published |

**Routing:**
- Fry owns Phase 1 implementation (Week 1–4)
- Professor + Nibbler gate Phase 1 before Phase 2 begins
- Bender signs off round-trip tests before Phase 1 ship gate
- Kif establishes BEIR baseline in Phase 3

**Why:** The spec is complete at v4. The team needs a stable execution sequence with clear gates so parallel work (implementation, tests, docs, review) stays coordinated. Front-loading the scaffold removes ambiguity for Fry before the first line of implementation code is written.

### 2026-04-13: Fry Sprint 0 revision — addressing Nibbler blockers

**By:** Fry

**What:**
Applied targeted fixes to Sprint 0 artifacts so the scaffold is internally coherent and proposals match actual CI behavior:

1. **Cargo.toml + src/main.rs coherence** — Added `env` feature to `clap`; replaced `~/brain.db` default with platform-safe `default_db_path()` function.
2. **CI / proposal alignment** — Removed musl/static-link gates from CI, moved to release-only. CI now matches proposal: `cargo fmt` + `cargo clippy` + `cargo check` + `cargo test`.
3. **release.yml hardening** — Fixed tag trigger glob pattern; pinned `cross` to version 0.2.5.
4. **Phase 1 OCC semantics** — Added explicit "Concurrency: Optimistic Concurrency Control" section with compare-and-swap, version bump, and MCP contract definition.
5. **knowledge_gaps privacy** — Replaced raw `query_text` with `query_hash` + conditional store; schema-default is privacy-safe.

**Why:** Closes gaps identified by Nibbler's adversarial review, ensuring scaffold passes its documented gate and all proposals internally cohere. No implementation logic added beyond minimum for platform safety.

### 2026-04-14: Adopt rust-best-practices skill as standing Rust guidance

**By:** Fry (recommended), macro88 (accepted)

**What:** Adopt the `rust-best-practices` skill (Apollo GraphQL public handbook, 9 chapters) as standing guidance for all Rust implementation and review work in this repo. Key chapters: borrowing vs cloning, clippy discipline, performance mindset, error handling, testing, generics, type-state, documentation, concurrency.

**Caveats:**
- `#[expect(...)]` requires MSRV ≥1.81; verify before enforcing (current `Cargo.toml` specifies `edition = "2021"` without explicit MSRV)
- `rustfmt.toml` import reordering (`group_imports = "StdExternalCrate"`) needs nightly; don't add until stable supports it or CI has a nightly-fmt step
- Snapshot testing (`insta`) recommended but defer to Phase 1 testing work, not before
- `Cow<'_, T>` useful in parsing but don't over-apply; prefer `&str`/`String` initially, refactor only if profiling shows benefit
- Dynamic dispatch and type-state pattern: overkill for current scope; revisit if plugin architecture or multi-step builder API emerges

**Why:** The skill directly aligns with GigaBrain's existing practices: error handling split (`thiserror` for `src/core/`, `anyhow` for CLI/main), CI discipline (`cargo fmt --check`, `cargo clippy -- -D warnings`), and performance constraints (single static binary, lean embedding/search pipeline). Provides consistent vocabulary for code review and implementation guidance.

**Decision:** Adopted. All agents writing or reviewing Rust should reference the SKILL.md quick reference before starting work.

### 2026-04-14: User directive — review Rust workspace skill and use consistently

**By:** macro88 (via Copilot)

**What:** Review the Rust-specific skill in the workspace and, if it is good, use it consistently when building Rust in this project.

**Why:** User request — captured for team memory. (Fry reviewed and recommended adoption — see above decision.)

### 2026-04-13: User directive — branch + PR workflow

**By:** macro88 (via Copilot)

**What:** Never commit directly to `main`. Always work from a branch, open a PR, link the PR to the relevant GitHub issue, and include the relevant OpenSpec proposal/change.

**Why:** User request — ensuring team memory captures governance requirement.

### 2026-04-14: Phase 1 OpenSpec Unblock

**By:** Leela  
**Date:** 2026-04-14  

**What:** Created the complete OpenSpec artifact set for `p1-core-storage-cli` to unblock `openspec apply`:
- `design.md` — technical design with 10 key decisions and risk analysis
- `specs/core-storage/spec.md` — DB init, OCC, WAL specs
- `specs/crud-commands/spec.md` — init, get, put, list, stats, tags, link, compact specs
- `specs/search/spec.md` — FTS5, SMS short-circuit, hybrid set-union merge specs
- `specs/embeddings/spec.md` — candle model init, embed, chunking, vector search specs
- `specs/ingest-export/spec.md` — import, export, ingest, markdown parsing, round-trip specs
- `specs/mcp-server/spec.md` — 5 core MCP tools, error codes, OCC over MCP
- `tasks.md` — 57 actionable tasks in 12 groups for Fry on `phase1/p1-core-storage-cli`

**Key Design Decisions:**
1. Single connection per invocation; WAL handles concurrent readers
2. Candle lazy singleton init via `OnceLock`; only embed/query pay cost
3. Model weights via `include_bytes!` (default offline; `online-model` feature for smaller builds)
4. Hybrid search: SMS → FTS5+vec → set-union merge (RRF switchable via config)
5. OCC error codes: CLI exit 1; MCP `-32009` with `current_version` in data
6. Room-level palace filtering deferred to Phase 2; wing-only in Phase 1
7. CPU-only inference in Phase 1; GPU detection deferred to Phase 3
8. `thiserror` in core, `anyhow` in commands (standing team decisions)

**Scope Boundary:**

Phase 1 (Fry executes now):
- All CRUD commands, FTS5 search, candle embeddings, hybrid search
- Import/export, ingest with SHA-256 idempotency, round-trip tests
- 5 core MCP tools via rmcp stdio
- Static binary verification

Phase 2 (blocked on Phase 1 gate):
- Graph traversal, assertions, contradiction detection, progressive retrieval
- Palace room-level filtering, novelty checking
- Full MCP write surface

**Routing:** Fry (implementation), Professor (db.rs/search.rs/inference.rs review), Nibbler (MCP adversarial), Bender (round-trip tests), Scruffy (unit test coverage)

**Why:** All artifacts now complete; `openspec apply p1-core-storage-cli` ready. Phase boundary locked. Spec-driven execution can begin on branch `phase1/p1-core-storage-cli`.

### 2026-04-14: Phase 1 Foundation Slice — types.rs design decisions

**By:** Fry

**What:** Implemented `src/core/types.rs` (tasks 2.1–2.6) with foundational type system:
- `Page`, `Link`, `Tag`, `TimelineEntry`, `SearchResult`, `KnowledgeGap`, `IngestRecord` structs
- `SearchMergeStrategy` enum (SetUnion, Rrf)
- `OccError`, `DbError` enums (thiserror-derived)
- All gates pass: `cargo check`, `cargo clippy -- -D warnings`, `cargo fmt --check`

**Design Choices:**
1. **`Page.page_type` instead of `type`** — Rust keyword reserved; `#[serde(rename = "type")]` for JSON/YAML
2. **`HashMap<String, String>` for frontmatter** — Simple string-to-string; upgrade to `serde_yaml::Value` if nested structures needed later
3. **`Link` uses slugs, not page IDs** — DB layer resolves to IDs internally; type system stays user-facing
4. **`i64` for all integer IDs/versions** — Matches SQLite INTEGER (64-bit signed)
5. **Module-level `#![allow(dead_code)]`** — Temporary; remove when db.rs wires types
6. **`SearchMergeStrategy::from_config`** — Parses config table strings with SetUnion default (fail-safe)

**Why:** Small but team-visible choices affecting how every module imports core types. Documented now to prevent re-litigation per-file.

### 2026-04-14: User directive (copilot) — main protection enabled

**By:** macro88 (via Copilot)

**What:** Main branch is now protected. All commits must flow through branch → PR → review → merge workflow.

**Why:** User request — ensuring branch hygiene and team consensus on all changes.

### 2026-04-14: DB Layer Implementation — T02 database.rs slice

**By:** Fry

**What:** Completed `src/core/db.rs` with sqlite-vec auto-extension registration, schema DDL application, and error type alignment:
1. **sqlite-vec** loaded via `sqlite3_auto_extension(Some(transmute(...)))` (process-global, acceptable for single-binary CLI)
2. **Schema DDL** applied as-is from `schema.sql` via `execute_batch`, preserving PRAGMAs (WAL, foreign_keys)
3. **Error types** use `thiserror::Error` for `DbError` (core/ layer boundary; MCP layer handles conversion to anyhow)
4. **Link schema** uses integer FKs (`from_page_id`, `to_page_id`) internally; struct resolves slugs at app layer

**Why:** Foundation-level plumbing. These choices propagate to markdown parsing (T03), search (T04), and MCP (T08). Documented now to prevent re-alignment work downstream.

**Status:** Validated. Tests pass. `cargo check/clippy/fmt` clean on branch `phase1/p1-core-storage-cli`.

### 2026-04-14: Link Contract Clarification — slugs at app layer, IDs in DB

**By:** Leela (Lead)

**What:** Resolved ambiguity between schema (`from_page_id`, `to_page_id` integers) and task spec (`from_slug`, `to_slug` strings). Decision: **slugs are the correct app-layer contract**.
- `Link` struct holds `from_slug` and `to_slug` (application-layer identity, stable across schema migrations)
- DB layer resolves slugs to page IDs on insert (`SELECT id FROM pages WHERE slug = ?`)
- DB layer reverses join on read (`SELECT * FROM links WHERE from_page_id = ? ...` then resolve IDs back to slugs)
- Callers (CLI, MCP) never see integer page IDs

**Corrections Applied (data-loss bugs):**
1. `Link.context: String` — was missing from struct; schema has it. Added to prevent silent data loss on round-trip.
2. `Link.id: Option<i64>` — was `i64` (sentinel value problem). Changed to Option; `None` before insert, `Some(id)` after.
3. `Page.truth_updated_at` and `Page.timeline_updated_at` — both missing from struct. Added to support incremental embedding (stale chunk detection).

**Why:** Standard view/data model separation. Slugs are the stable external identity (used in CLI, MCP, docs). Integer IDs are DB-layer plumbing for referential integrity and index performance.

**Routing:** Fry must use corrected `Link` and `Page` fields in all db.rs read/write paths (T03+). Bender's validation checklist updated.

**Status:** Unblocked. No architectural changes needed. Type corrections applied.

### 2026-04-14: Phase 1 Foundation Validation Plan — Bender's checklist (anticipatory)

**By:** Bender (Tester)

**What:** Authored comprehensive validation checklist for tasks 2.1–2.6 (type system) before code lands. Minimum useful checks:
- Schema–struct field alignment (all 16 `pages` columns mapped to `Page` fields; all 8 `links` columns mapped to `Link` fields)
- Error enum hygiene (`OccError::Conflict { current_version }` variant, `thiserror` not `anyhow`)
- `SearchMergeStrategy` exhaustiveness (exactly `SetUnion` and `Rrf`)
- `type` keyword handling (Rust reserved; must rename to `page_type` with serde remap)
- Edge cases: empty slugs, version = 0, frontmatter type stability, timestamp format validation

**Execution:** After Fry lands T02–T06, run `cargo check` (hard gate), diff struct fields against schema columns, verify error types, confirm compile gate passes.

**Estimated time:** 15 minutes once code lands.

**Status:** Plan ready, waiting on code.

### 2026-04-14: Phase 1 Markdown Slice — T03 decisions

**By:** Fry

**What:** Completed `src/core/markdown.rs` with four foundational parsing/render decisions:
1. **Frontmatter keys render alphabetically** — Deterministic output for byte-exact round-trip. Canonical format: unquoted YAML values, sorted keys.
2. **Timeline separator omit-when-empty** — No spurious `\n---\n` for empty timelines; `split_content` already handles zero-separator case (returns empty timeline).
3. **YAML parse graceful degradation** — Returns `(HashMap<String, String>, String)` with no `Result`. Malformed YAML → empty map; body still extracted.
4. **Non-scalar YAML skip** — Sequences and mappings dropped; HashMap<String, String> contract holds scalars only. Tags stored separately in `tags` table.

**Implications for downstream:**
- **Bender:** `roundtrip_raw.rs` fixtures must use canonical format (alphabetically sorted frontmatter keys) to pass byte-exact gate.
- **Professor:** No review needed; pure text parsing layer with no DB/search impact.
- **Leela:** T03 complete; T04 (palace.rs) now unblocked.

**Why:** Small but team-visible choices affecting every downstream module. Locked in before Bender writes test fixtures to prevent re-litigation per-file.

**Status:** All gates pass. Code on branch `phase1/p1-core-storage-cli`. Ready for integration.

### 2026-04-14: Rust skill standing guidance — adoption decision

**By:** Fry (recommended), macro88 (accepted)

**What:** Adopt `rust-best-practices` skill (Apollo GraphQL public handbook) as standing Rust guidance. Key emphases for GigaBrain:
- **Borrowing:** Prefer borrowing and slices/`&str` at API boundaries unless ownership required
- **Error handling:** `Result`-based errors; reserve `anyhow` for binary-facing orchestration; typed errors for library surfaces
- **Clippy:** Use as standing gate; prefer local `#[expect(clippy::...)]` with rationale over `#[allow]`
- **Comments:** Focus on why, safety, workarounds, or linked design decisions
- **Performance:** Measurement-first; avoid unnecessary cloning

**Standing guidance for this repo (required):**
- Borrowing and slices/`&str` at API boundaries
- Treat unnecessary cloning, panic-based control flow, and silent lint suppression as review smells
- Use Clippy as standing gate
- Keep comments focused on rationale

**Optional guidance (use as heuristic, not law):**
- Type-state pattern, snapshot testing (`insta`), `#![deny(missing_docs)]`, pedantic Clippy groups, `Cow`-based API design

**Caveats:**
- `#[expect(...)]` requires MSRV ≥1.81 (current `Cargo.toml` is `edition = "2021"` without explicit MSRV; verify before enforcing)
- `rustfmt.toml` import reordering (`group_imports`) uses nightly syntax; defer until stable or CI has nightly step
- Snapshot testing deferred to Phase 1 testing work
- `Cow<'_, T>` useful in parsing but avoid over-application; refactor only if profiling shows benefit
- Type-state and dynamic dispatch overkill for current scope; revisit if architecture emerges

**Why:** Aligns with GigaBrain's existing practices (error handling split, CI discipline, performance constraints). Provides consistent vocabulary for code review.

**Decision:** Adopted. All agents writing or reviewing Rust should reference the SKILL.md quick reference before starting work.

### 2026-04-14: Phase 1 markdown test strategy — test expectations locked

**By:** Scruffy

**What:** Prepared comprehensive unit test expectations for T03 before Fry writes parsing logic. Organized by function with minimum must-cover cases:

**parse_frontmatter (5 must-cover cases):**
1. Parses string scalar frontmatter when file starts with bare `---` boundary
2. Returns empty map and full body when opening boundary missing
3. Treats leading newline before boundary as no frontmatter
4. Accepts empty frontmatter block
5. Stops at first closing bare boundary

**split_content (5 must-cover cases):**
1. Splits on first bare boundary line
2. Returns full body and empty timeline when boundary missing
3. Only splits once when timeline contains additional boundaries (later `---` stays inside)
4. Does not split on horizontal rule variants (` ---`, `--- `, `----`)
5. Preserves newlines around sections without trimming

**extract_summary (4 must-cover cases):**
1. Returns first non-heading non-empty paragraph
2. Falls back to first line when no paragraph exists
3. Caps summary at 200 chars deterministically
4. Ignores leading blank lines

**render_page (4 must-cover cases):**
1. Renders frontmatter, compiled truth, and timeline in canonical order
2. Parse-render-parse is idempotent for canonical page
3. Renders empty timeline deterministically
4. Renders empty frontmatter deterministically

**Fixture guidance:**
- Canonical fixture: standard frontmatter + heading + paragraph + timeline
- Boundary trap: proves split only cuts once
- No-frontmatter: proves parse fallback is lossless

**Critical implementation traps:**
- HashMap order nondeterministic (must sort for canonical output)
- Do not trim() away fidelity (breaking raw round-trip)
- Frontmatter type coercion underspecified (use string-scalar fixtures only in Phase 1)
- Two different `---` roles exist (frontmatter delimiters vs compiled-truth/timeline split)

**Why:** Locks expectations before code lands, preventing re-writing tests per-function. Prevents markdown round-trip from drifting in Phase 2.

**Status:** Strategy prepared. Test module ready once Fry lands code.

### 2026-04-14: T03 Markdown Slice — Bender approval with two non-blocking concerns

**By:** Bender (Tester)  
**Status:** APPROVED

**What:** Reviewed `src/core/markdown.rs` (commit `0ae8a46`) against all spec invariants. All 4 public functions (`parse_frontmatter`, `split_content`, `extract_summary`, `render_page`) match spec; 19/19 unit tests pass; no violations found.

**Approval Decision:** Ship T03 as complete.

**Non-blocking Concerns (Documented for future phases):**

1. **Naive YAML rendering loses structured values (Phase 2 hardening)**
   - Impact: Data loss on round-trip for non-scalar frontmatter
   - Current mitigation: Phase 1 uses string-scalar frontmatter only; HashMap<String, String> type constraint enforced
   - Phase 2 action: Fry should use `serde_yaml::to_string()` for frontmatter serialization when values can originate from user input

2. **No lib.rs — integration tests blocked (Phase 1 gate blocker)**
   - Impact: `tests/roundtrip_semantic.rs` and `tests/roundtrip_raw.rs` cannot import core modules from external test files
   - Classification: Structural prerequisite, not a markdown.rs bug
   - Blocker level: Blocks Phase 1 ship gate (round-trip tests required)
   - Action: Fry must add `src/lib.rs` re-exporting `pub mod core` before round-trip integration tests can run

**Routing:** Fry: Log lib.rs gap and YAML serialization hardening as follow-up tasks; lib.rs is Phase 1 blocker.

### 2026-04-14: Phase 1 Init + Get Slice — T05, T07 implementation complete

**By:** Fry (Implementer)  
**Status:** COMPLETE

**What:** Implemented `src/commands/init.rs` (T05) and `src/commands/get.rs` (T07) — first two usable CLI commands.

**T05 init.rs decisions:**
1. Existence check before `db::open` prevents re-initialization of existing database
2. No schema migration on existing DBs; `init` is strictly creation-only

**T07 get.rs decisions:**
1. `get_page()` extracted as public helper for OCC reuse in T06 and beyond (no circular deps)
2. Frontmatter stored as JSON in schema; `get_page` deserializes with fallback to empty map on malformed JSON
3. `--json` output serializes full `Page` struct; default is canonical markdown via `render_page`

**Wiring:** main.rs already correct from Sprint 0 scaffold; no changes needed.

**Test coverage:**
- init: 3 tests (creation, idempotent re-run, nonexistent parent rejection)
- get: 4 tests (data round-trip, markdown render, not-found error, frontmatter deser)
- Total new: 7 tests; 48 tests pass overall (41 baseline + 7 new)

**Gates passed:** `cargo fmt --check` ✓, `cargo clippy -- -D warnings` ✓, `cargo test` ✓

**Integration points:**
- Bender: `get_page` available for round-trip test harness integration
- T06 (put): Can import `get_page` to read current version for OCC checks

### 2026-04-14: T06 put Command — Unit test coverage specification locked

**By:** Scruffy (Coverage Master)  
**Status:** BLOCKED — implementation not ready; coverage plan locked

**What:** Prepared comprehensive unit test specification for T06 `put` command before code lands. Three core test cases locked; coverage targets frozen to prevent drift.

**Required test cases (minimum):**

1. **Create path:** Insert version 1, derive fields from stdin markdown
   - Parse frontmatter + split content
   - Store title, page_type, summary, wing, room, compiled_truth, timeline
   - version = 1

2. **Update path (OCC success):** Compare-and-swap when expected version matches
   - Insert initial page at version = 1
   - Call put with `expected_version = Some(1)` and changed markdown
   - Update succeeds, version becomes 2, slug stable, content fully replaced

3. **Conflict path (OCC failure):** Reject stale version without mutation
   - Insert page at version = 2
   - Call put with `expected_version = Some(1)`
   - Returns conflict with `current_version = 2`, row unchanged, version remains 2

**Implementation seam required:**
- Pure helper: `put_page(&Connection, slug, raw markdown, expected_version) → Result<version | OccError>`
- CLI `run()` as thin wrapper: reads stdin, formats messages
- This enables deterministic unit coverage without fake terminal plumbing

**Assertion guards:**
1. Frontmatter: compare deserialized maps, not raw JSON string
2. Markdown split: assert exact truth/timeline values, boundary newlines
3. OCC semantics: stale version must fail without row mutation
4. Phase 1 room: stored as empty string even when headings exist

**Test naming:**
- `put_creates_page_from_stdin_markdown_with_version_one`
- `put_updates_existing_page_when_expected_version_matches`
- `put_returns_conflict_and_preserves_row_when_expected_version_is_stale`
- `put_derives_summary_wing_and_room_from_markdown_and_slug` (can fold into create)

**Status:** Ready for implementation. Specification locked; awaiting Fry's code land.

## Governance

- All meaningful changes require team consensus
- Document architectural decisions here
- Keep history focused on work, decisions focused on direction
- OpenSpec proposals are created before implementation; decisions.md records accepted direction and lasting team rules
- Never commit directly to `main`; all changes flow through branch → PR → review → merge
