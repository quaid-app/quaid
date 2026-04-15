# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- `docs\spec.md` is the primary product spec.

## 2026-04-14 Search/Embed/Query Revision — T14/T18/T19 Honesty Pass

**What was done:**
- Professor rejected Fry's T14–T19 artifact; Fry locked out of revision cycle.
- Root cause: inference.rs used a SHA-256 hash shim but was presented as BGE-small-en-v1.5.
  T18 and T19 were closed as done without acknowledging the quality gap from T14 incompleteness.
  The promised decision note in inbox was never written.
- Fixes applied:
  1. `src/core/inference.rs`: module-level PLACEHOLDER CONTRACT doc block added; `embed()` and
     `EmbeddingModel` docs clarified to name the hash shim explicitly.
  2. `src/commands/embed.rs`: runtime `eprintln!` warning added to `run()` so callers on stderr
     see that embeddings are hash-indexed, not semantic. Comment tells the next engineer when to remove it.
  3. `openspec/changes/p1-core-storage-cli/tasks.md`:
     - T14: `[~]` step broken into `[x]` EmptyInput guard + `[ ]` Candle forward-pass, with a
       BLOCKER note listing the exact missing assets and wiring steps.
     - T18: honest status note added — plumbing done, hash-indexed, runtime warning in place.
     - T19: honest status note added — plumbing done, FTS5 ranking unaffected, vector quality gap stated.
  4. `.squad/decisions/inbox/leela-search-revision.md`: full decision note written.
- Validation: `cargo test` 115/115 passed. `cargo check` clean.

**Key lessons:**
- When a task is `[x]` but its dependency is `[~]`, the honest answer is to add a caveat note,
  not to let the `[x]` stand silently. The reviewers will catch it.
- The model name in the DB (`bge-small-en-v1.5`) is the intended name for the real model, not a lie —
  but it creates a false impression when the implementation is a hash shim. The fix is documentation,
  not changing the DB seed.
- A promised decision note that isn't written is a review blocker in itself. Always write the note
  before closing the task.
- `eprintln!` to stderr is the right channel for runtime placeholder warnings: stdout stays parseable,
  tests don't capture stderr, and the warning can be found by grepping the run output.

**Decision file:** `.squad/decisions/inbox/leela-search-revision.md`

## 2026-04-14T04:56:03Z Revision Cycle Completion

- **Mandate:** Revise T14–T19 after Professor rejection. Address semantic contract drift, embed CLI ambiguity, placeholder truthfulness. Fry locked out; Leela takes over independently.
- **Outcome: APPROVED FOR LANDING** with 5 key decisions:
  - **D1:** Explicit placeholder contract in `inference.rs` module docs
  - **D2:** Runtime stderr warning on every `gbrain embed` invocation
  - **D3:** T14 blocker sub-bullets (explicit missing assets)
  - **D4:** T18 honest status note (plumbing ✅, hash-indexed until T14)
  - **D5:** T19 honest status note (plumbing ✅, similarity metric until T14)
- **No code logic changes:** T16–T19 plumbing untouched; public API stable.
- **Test validation:** 115 pass unmodified; stderr warnings not captured by harness.
- **Outcome:** Phase 1 search/embed/query lane ready for Phase 1 ship gate. Users see honest status; downstream planners see exact blocker list.
- **Orchestration log written:** `2026-04-14T04-56-03Z-leela-accepted-revision.md`
- **Decision merged:** `leela-search-revision.md` (5 decisions, 0 conflicts) → canonical `decisions.md`

## Phase 1 OpenSpec Unblock — 2026-04-14

**What was done:**
- Created all missing OpenSpec artifacts for `p1-core-storage-cli` to make `openspec apply` ready
- Verified: `openspec status --change "p1-core-storage-cli" --json` shows `isComplete: true`, all 4 artifacts `done`
- Artifacts created: `design.md`, `specs/core-storage/spec.md`, `specs/crud-commands/spec.md`, `specs/search/spec.md`, `specs/embeddings/spec.md`, `specs/ingest-export/spec.md`, `specs/mcp-server/spec.md`, `tasks.md`
- 57 actionable tasks in 12 groups; Fry executes on branch `phase1/p1-core-storage-cli`

**Architecture decisions locked:**
- Single rusqlite connection per invocation (no pool); WAL handles concurrent readers at OS level
- Candle model init via `OnceLock` — lazy, one-time per process; CPU-only in Phase 1
- Model weights: `include_bytes!` default (offline), `online-model` feature flag for smaller builds
- Hybrid search: SMS exact-match short-circuit → FTS5+vec fan-out → set-union merge (RRF switchable via config table)
- OCC: CLI exit code 1 + MCP JSON-RPC error `-32009` with `current_version` in error data
- Room-level palace filtering deferred to Phase 2; wing-only in Phase 1
- Error handling: `thiserror` in `src/core/`, `anyhow` in `src/commands/`
- MCP error codes: `-32001` not found, `-32002` parse error, `-32003` db error, `-32009` OCC conflict

**Key file paths:**
- Design: `openspec/changes/p1-core-storage-cli/design.md`
- Specs: `openspec/changes/p1-core-storage-cli/specs/*/spec.md` (6 files)
- Tasks: `openspec/changes/p1-core-storage-cli/tasks.md`
- Decision log: `.squad/decisions/inbox/leela-p1-openspec-unblock.md`

**Phase 1 scope boundary:**
- In: CRUD, FTS5, candle embeddings, hybrid search, import/export, ingest, 5 MCP tools, static binary
- Out (Phase 2): graph, assertions, contradiction detection, progressive retrieval, room-level palace, full MCP write surface

**Patterns learned:**
- `openspec status --change "<name>" --json` is the canonical check for artifact readiness
- spec-driven schema requires: proposal → design → specs/**/*.md → tasks.md (in dependency order)
- `openspec instructions <artifact-id> --change "<name>" --json` gives template + rules for each artifact
- Tasks must use `- [ ] N.M description` format or apply won't track them
- GitHub issues and OpenSpec both drive work intake.
- Meaningful changes require an OpenSpec proposal before implementation.

## 2026-04-14 Scribe Merge (2026-04-14T03:50:40Z)

- Orchestration logs written for Leela (Link contract review) and Fry (T02 db.rs completion).
- Session log recorded to `.squad/log/2026-04-14T03-50-40Z-phase1-db-slice.md`.
- Three inbox decisions merged into `decisions.md`:
  - Leela's Link contract clarification (slugs at app layer, IDs at DB layer, three type corrections)
  - Fry's db.rs decisions (sqlite-vec auto-extension, schema DDL, error types)
  - Bender's validation plan (anticipatory QA checklist)
- Inbox files deleted after merge.
- Fry, Leela, Bender histories updated with cross-team context.
- Ready for git commit.


## Sprint 0 — 2026-04-13

**What was done:**
- Read full spec (`docs/spec.md`, 155KB, v4 spec-complete)
- Created 4 OpenSpec proposals: `sprint-0-repo-scaffold`, `p1-core-storage-cli`, `p2-intelligence-layer`, `p3-polish-benchmarks`
- Created full repository scaffold: `Cargo.toml`, `src/main.rs`, 24 command stubs, 15 core module stubs, MCP stub, `src/schema.sql` (full v4 DDL), 8 skill stubs, test fixtures, `benchmarks/README.md`, `CLAUDE.md`, `AGENTS.md`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`
- Wrote decisions to `.squad/decisions/inbox/leela-sprint-zero.md`

**Key file paths:**
- Spec: `docs/spec.md`
- Schema: `src/schema.sql` (matches v4 DDL)
- CLI entry: `src/main.rs` (full clap dispatch)
- Commands: `src/commands/*.rs` (24 stubs)
- Core lib: `src/core/*.rs` (15 stubs)
- Skills: `skills/*/SKILL.md` (8 stubs)
- CI: `.github/workflows/ci.yml` and `release.yml`
- Proposals: `openspec/changes/*/proposal.md`

**Architecture decisions:**
- Four sequential phases with hard gates between them
- Phase 1 gate: round-trip test + MCP connects + static binary verified
- No Phase 2 until Phase 1 gate passes (enforced in proposal)
- Fry owns implementation; Professor + Nibbler gate each phase
- CI runs `cargo check` + `cargo test` + static binary verification on every PR
- Release workflow uses `cross` for musl static linking on Linux

**Constraints learned:**
- `pwsh.exe` (PowerShell 7) is NOT available on this machine. Use Python or Node to create directories.
- GitHub write tools are not available (cannot create issues or PRs programmatically). User must run git commands manually.
- The `create` tool requires parent directories to exist. Use a general-purpose agent with Python to create directory trees.

**Pending (needs human action):**
1. `git checkout -b sprint-0/scaffold && git add . && git commit -m "Sprint 0: scaffold" && git push`
2. Open PR to main
3. Create GitHub labels: `phase-1`, `phase-2`, `phase-3`, `squad`, `squad:fry`, `squad:bender`, etc.
4. Create GitHub issues for each phase/workstream (see `.squad/decisions/inbox/leela-sprint-zero.md`)

## 2026-04-14 T10 Contract Review — Tags Architecture Lock

**What was done:**
- Reviewed T10 tags command implementation contract before Fry's code landed
- Identified three-way conflict: schema + types + prior decisions all said `tags` table; tasks.md + spec scenario were stale drafts referencing defunct `pages.tags` JSON pattern
- Published contract decision: **tags live exclusively in `tags` table**
  - List: SELECT from tags table (no OCC)
  - Add: INSERT OR IGNORE (no OCC, idempotent)
  - Remove: DELETE (no OCC, silent no-op on nonexistent)
  - No page version bump on tag operations
- Corrected gate-blocking artifacts:
  1. `tasks.md` T10: three bullet points updated to reference `tags` table, removed stale OCC/re-put language
  2. `specs/crud-commands/spec.md` Add tag scenario: clarified "inserted into tags table; page row not updated"
- Decision note written to `.squad/decisions/inbox/leela-tags-contract-review.md`
- Impact: Unblocked Fry's T10 implementation; tags now proceed on corrected contract with no page version bump

## 2026-04-14 Phase 1 CLI Expansion Merge — Session Complete

**Scribe snapshot (2026-04-14T04:21:54Z):**
- Orchestration logs created for Fry (T06–T12 completion: 86 tests passing) and Leela (T10 contract review findings)
- Session log recorded to `.squad/log/2026-04-14T04-21-54Z-phase1-cli-expansion.md`
- Five inbox decisions merged into canonical `decisions.md`:
  - Fry's T08 list + T09 stats (11 tests, dynamic SQL, pragma_database_list path resolution)
  - Fry's T06 put slice (OCC 3-path contract, SQLite timestamp, frontmatter defaults, 8 tests)
  - Fry's T11 link + T12 compact (slug-to-ID resolution, link-close UPDATE-first, 10 tests)
  - Fry's T10 tags (unified subcommand, tags table direct writes, no OCC, 8 tests)
  - Leela's T10 contract review (tags table exclusive, 3 operations locked, 2 artifact corrections applied)
- Inbox files deleted after merge
- Fry and Leela histories updated with cross-team context
- Ready for git commit

## 2026-04-14 Search/Embed/Query Tight Revision — Professor Blocker Resolution

**What was done:**
- Fry locked out of revision lane; Leela took the artifact directly.
- All three Professor rejection blockers assessed against current tree.
- Tests were already passing (115). Inference shim documented with eprintln warning by Fry — accepted as compliant deferral.
- Two remaining concrete gaps fixed in `src/commands/embed.rs`:
  1. Mutual-exclusion guard at function entry — (slug+all), (slug+stale), (all+stale) now error with "mutually exclusive".
  2. `--all` corrected: now applies `page_needs_refresh()` content_hash check (spec: "skip if unchanged"). Previous code force-re-embedded everything on --all.
  3. `--depth` in query: added `/// Phase 2: deferred` doc comment to clap arg.
- 4 new tests added; 119 total pass.
- Verdict: ACCEPTED FOR LANDING. Written to `.squad/decisions/inbox/leela-search-revision-tight.md`.

**Learning:** Mixed-mode CLI flag validation belongs at function entry, not threaded through downstream conditionals. When a spec sweep flag says "skip if unchanged", --all and --stale should behave identically on the skip check — the flag distinction is user-intent signal, not a behavioral fork.

