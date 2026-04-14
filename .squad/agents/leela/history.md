# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- `docs\spec.md` is the primary product spec.

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
