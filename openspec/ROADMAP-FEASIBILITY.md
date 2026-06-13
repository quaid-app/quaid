# Roadmap feasibility verdicts (#73, #76, #136, #167, #173)

These five issues previously had no review coverage and no feasibility signal.
The canonical per-issue verdicts — with the **blocking primitive** for each, so a
future change touching that primitive re-triggers review — live in the roadmap:

→ `docs/roadmap_v3.md`, section **"Roadmap feasibility verdicts (#73, #76, #136, #167, #173)"**.

Summary (read the roadmap table for evidence and file:line references, accurate
as of commit `92b9018`):

- **#73** (job queue) — *Mostly already built.* Two SQLite-backed queues exist
  (`extraction_queue`, `embedding_jobs`) with lease/retry/status. Generalize into
  one `jobs` table; do not build a third. **Blocker:** no generic `jobs` table
  (hardcoded payloads, `CHECK`-constrained `trigger_kind`, no `quaid jobs`/MCP surface).
- **#76** (REFRAG compression) — *Infeasible as written.* Closed-decoder MCP
  returns text; dense-embedding splicing has nowhere to land. Fold into
  `retrieval-quality-rerank` as **extractive compression** (#76 already deferred there).
  **Blocker:** closed-decoder MCP boundary.
- **#167** (image-to-memory) — *Blocked.* No watcher→jobs ingest hook for non-md
  files; the proposed "skill intercepts via watcher event" hook does not exist.
  **Blocker:** needs **#73** first.
- **#136** (active enrichment) — *Heaviest lift.* Entity extraction is regex-only,
  5 ms-budgeted, writes no `links` rows; no entity→pages index or ingest fan-out.
  **Blocker:** entity→pages index + ingest fan-out (and #107/#72 durable edges, #73 propagation).
- **#173** (git-sync) — *Mostly feasible; spec gaps first.* DB-only state (links,
  `raw_data`, contradictions, gaps) does not travel via git, and `raw_imports`
  byte-exact restore is per-machine, so `collection restore` over a checkout
  diverges from git HEAD. **Blocker:** DB-only-state representation + restore-vs-checkout
  semantics (and the duplicate-uuid halt must be fixed first).

**Sequencing:** #73 first (unlocks #167 and #136), fold #76 into rerank, spec
#173's DB-only-state and restore-vs-checkout semantics before any `quaid sync`.

When a follow-on change proposal lands for any of these (e.g. a `jobs` table
generalizing `queue.rs`), create a normal `openspec/changes/<change-id>/`
proposal and reference this note as the feasibility input.
