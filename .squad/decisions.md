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

## Governance

- All meaningful changes require team consensus
- Document architectural decisions here
- Keep history focused on work, decisions focused on direction
- OpenSpec proposals are created before implementation; decisions.md records accepted direction and lasting team rules
- Never commit directly to `main`; all changes flow through branch → PR → review → merge
