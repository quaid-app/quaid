# Squad Team

> gigabrain

## Coordinator

| Name | Role | Notes |
|------|------|-------|
| Squad | Coordinator | Routes work, enforces handoffs, reviewer gates, and OpenSpec-first execution. |

## Members

| Name | Role | Charter | Status |
|------|------|---------|--------|
| Leela | Lead | `.squad/agents/leela/charter.md` | ✅ Active |
| Fry | Main Engineer | `.squad/agents/fry/charter.md` | ✅ Active |
| Bender | Tester | `.squad/agents/bender/charter.md` | ✅ Active |
| Amy | Technical Writer | `.squad/agents/amy/charter.md` | ✅ Active |
| Hermes | Docs Site Engineer | `.squad/agents/hermes/charter.md` | ✅ Active |
| Zapp | DevRel / Growth | `.squad/agents/zapp/charter.md` | ✅ Active |
| Professor | Code Peer Reviewer | `.squad/agents/professor/charter.md` | ✅ Active |
| Nibbler | Adversarial Reviewer | `.squad/agents/nibbler/charter.md` | ✅ Active |
| Scruffy | Unit Test Master | `.squad/agents/scruffy/charter.md` | ✅ Active |
| Kif | Benchmark Expert | `.squad/agents/kif/charter.md` | ✅ Active |
| Mom | Edge Case Expert | `.squad/agents/mom/charter.md` | ✅ Active |
| Scribe | Session Logger | `.squad/agents/scribe/charter.md` | ✅ Active |
| Ralph | Work Monitor | `.squad/agents/ralph/charter.md` | ✅ Active |

## Project Context

- **Owner:** macro88
- **Project:** GigaBrain — a Rust personal knowledge brain with local-first search, ingest, and MCP support.
- **Primary spec:** `docs\spec.md`
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Work intake:** GitHub issues plus OpenSpec change proposals
- **Created:** 2026-04-13

## Issue Source

- **GitHub repository:** `macro88/gigabrain`
- **OpenSpec workspace:** `openspec\`
- **Routing rule:** GitHub issues track work intake; OpenSpec proposals define intended changes before implementation.

## Model Policy

| Agent | Preferred model | Notes |
|-------|-----------------|-------|
| Fry | `claude-opus-4.6` | Main implementation owner |
| Bender | `claude-opus-4.6` | Primary tester / destructive validation |
| Leela | `claude-sonnet-4.6` | Lead coordination and design review |
| Amy | `claude-sonnet-4.6` | Technical writing |
| Hermes | `claude-sonnet-4.6` | Docs site engineering |
| Zapp | `claude-sonnet-4.6` | DevRel, launch, growth |
| Professor | `gpt-5.4` | Code peer review |
| Nibbler | `gpt-5.4` | Adversarial review |
| Scruffy | `gpt-5.4` | Unit test depth and coverage |
| Kif | Requested: `Gemini 3.1 Pro` | Exact model not available on this CLI surface; keep role and intent visible |
| Mom | Requested: `Gemini 3.1 Pro` | Exact model not available on this CLI surface; keep role and intent visible |
| Scribe | `claude-haiku-4.5` | Logging and decision merging |
| Ralph | `claude-haiku-4.5` | Monitoring and backlog flow |

## Working Agreements

1. Every meaningful code, docs, benchmark, or site change starts with an OpenSpec change proposal that follows the local instructions in `openspec\`.
2. Scribe logs and merges decisions after work; Scribe does not replace the OpenSpec proposal step.
3. `docs\spec.md` is the core product spec until superseded by accepted OpenSpec changes.

## 2026-04-29T21:29:11+08:00 — Zapp Release v0.12.0 Complete

Zapp release lane completed successfully. Schema v7 finalized. Release workflow green. Tag v0.12.0 published to origin. See orchestration-log/ for details.

