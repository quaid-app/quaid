# Project Context

- **Owner:** macro88
- **Project:** GigaBrain — local-first Rust knowledge brain
- **Stack:** Rust, rusqlite, SQLite FTS5, sqlite-vec, candle + BGE-small-en-v1.5, clap, rmcp
- **Created:** 2026-04-13T14:22:20Z

## Learnings

- Edge-case work is an explicit part of this squad, not an afterthought.
- The requested target model is Gemini 3.1 Pro when available on the active surface.
- Proposal-first work makes it easier to identify which assumptions deserve stress.

## 2026-04-15 Graph Temporal Gate Fix Resolution

- **Mom's edge-case note** on future-dated links was identified as part of initial graph slice review (directionality contract blockers).
- **Temporal gate gap:** The original graph query only checked `valid_until >= today` but did not gate `valid_from <= today`, which allowed future-dated links to appear in the "active" graph.
- **Resolution:** Leela's graph slice revision (tasks 1.1–2.5) incorporated the fix into decision D2. Active temporal filter now enforces:
  ```sql
  (l.valid_from IS NULL OR l.valid_from <= date('now'))
  AND (l.valid_until IS NULL OR l.valid_until >= date('now'))
  ```
- **Status:** INCORPORATED. Graph slice approved for landing on `phase2/p2-intelligence-layer` 2026-04-15T23:15:50Z.
- **Lessons:** Edge-case work is most effective when it surfaces during contract-review blockers, not during post-landing firefighting. Mom's temporal concern directly influenced the final graph design.

## 2026-04-17 Phase 3 MCP Rejection Fixes (brain_raw + brain_gap + pipe)

- **Context:** Fry's Phase 3 MCP implementation was rejected by Nibbler on four specific grounds. Mom assigned as revision author while Fry is locked out of this cycle.
- **Fixes shipped:**
  - `brain_raw` now rejects non-object payloads (array/scalar) with `-32602`.
  - `brain_raw` now has an `overwrite: Option<bool>` field; silent `INSERT OR REPLACE` is blocked — returns `-32003` conflict if `overwrite` is not explicitly `true`.
  - `brain_gap` now caps `context` at 500 characters (`MAX_GAP_CONTEXT_LEN`) to prevent privacy leakage through the context sidecar.
  - `gbrain pipe` now blocks JSONL lines exceeding 5 MB (`MAX_LINE_BYTES`), emitting an error per oversized line and continuing — no process crash.
- **Tests added:** 7 new targeted edge-case tests covering all four rejection points plus boundary conditions.
- **All 282 tests pass. Clippy clean.**
- **Task 8.2 left pending** — Nibbler re-review required before it can close.
- **Decision record:** `.squad/decisions/inbox/mom-phase3-mcp-fixes.md`
- **Lesson:** The `INSERT OR REPLACE` pattern is a latent data-loss hazard. Any store-to-keyed-table operation should require an explicit opt-in for destructive replacement. The context-as-privacy-vector risk is subtle but real — bounded fields are the right default for any input that touches the privacy model.

---

## 2026-04-16 Phase 3 Task 8.2 — MCP Edge-Case Fixes (mom-phase3-mcp-fixes)

**Session:** mom-phase3-mcp-fixes (2309s, claude-sonnet-4.6)  
**Timestamp:** 2026-04-16T07:20:47Z

**What happened:**
- Task 8.2 REVISION COMPLETE: Addressed all four Nibbler Phase 3 MCP review blockers.
  - Decision D-M1: `brain_raw` data field restricted to JSON objects only. Non-objects rejected with `-32602`.
  - Decision D-M2: `brain_raw` now requires explicit `overwrite=true` to replace existing `(page_id, source)` rows. Silent replacement blocked; returns `-32003` conflict error with guidance.
  - Decision D-M3: `brain_gap` context capped at 500 characters. Longer values rejected with `-32602`. Prevents privacy leakage through context sidecar.
  - Decision D-M4: `gbrain pipe` blocks oversized JSONL lines at 5 MB (`MAX_LINE_BYTES`). Emits error per line, continues processing — no process crash.
- 4 decisions merged to `decisions.md`.
- 7 targeted tests added. All 282 tests pass. Clippy clean.
- Orchestration log written.
- **Status:** Task 8.2 left for re-review by different reviewer per phase 3 workflow (Nibbler).

**Next:** Await Nibbler re-review of all fixes before closing task 8.2.
