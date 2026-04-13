# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- `docs\spec.md` is the primary product spec.
- GitHub issues and OpenSpec both drive work intake.
- Meaningful changes require an OpenSpec proposal before implementation.

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
