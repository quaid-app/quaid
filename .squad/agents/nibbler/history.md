# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Phase 3 review confirmed that raw-data and gap endpoints are only acceptable once payload shape checks, overwrite intent, and transport-size caps are all closed together; one missing seam keeps the whole surface soft.
- Adversarial review begins at the proposal, not only at the code diff.
- This project values hidden-risk discovery and reviewer lockout discipline.
- Local-first systems still need security and misuse thinking.
- Privacy-safe fields are not enough if adjacent free-form fields can still carry the same secret.
- Line-oriented shell protocols need explicit payload caps or raw-data endpoints become an easy memory-pressure path.
- For vault-bound walks, `WalkBuilder` output is only a candidate list; root-bounded `open_root_fd` + `walk_to_parent` + `stat_at_nofollow` must be the only authority for classification if symlink escapes are to stay closed.

## 2026-04-15 In Progress

- Conducting final adversarial re-review of Phase 2 graph slice (tasks 1.1–2.5) after Scruffy cycle/self-loop suppression fix (commit `acd03ac`).
- Cross-team status: Professor completed parent-aware tree rendering (commit `44ad720`). Both commits now validated against graph specs. Awaiting Nibbler re-review completion before Phase 2 sign-off.


---

## 2026-04-16: Phase 3 Core Review — Rejection (task 8.2)

**Scope:** brain_gap, brain_gaps, brain_stats, brain_raw, call/pipe failure modes  
**Status:** Completed with REJECTION  

**Blocked artifacts:**
1. `src/mcp/server.rs` — brain_raw contract violation, no size limit, silent overwrites, gap privacy leak
2. `src/commands/pipe.rs` — oversized line handling

**Blocking findings:**
- brain_raw accepts non-object payloads (spec violation)
- No payload size limit (abuse vector)
- Silent replace semantics (data-loss risk)
- brain_gap context unbounded (privacy bypass seam)

**Decision:** nibbler-phase3-core-review.md merged to decisions.md  
**Task 8.2:** Not marked complete; different revision author required (reviewer lockout).

