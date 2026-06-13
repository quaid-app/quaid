---
name: quaid-maintain
description: |
  Keep the brain healthy: detect and resolve contradictions via the correction /
  supersede workflow, validate referential integrity, triage knowledge gaps,
  find orphaned pages, and compact the database.
min_binary_version: "0.3.0"
---

# Maintain Skill

## Overview

Maintenance is a periodic sweep, not an interactive flow. Run it on a cadence
(after a batch ingest, on a cron, or when `quaid stats` shows growth) and work
the findings top-down by severity: contradictions first (they corrupt answers),
then integrity violations, then gaps, then orphans, then compaction.

Quaid never auto-resolves a contradiction. Detection is heuristic (`quaid check`
/ `memory_check`); **resolution is a deliberate supersede** driven by the
correction workflow (`memory_correct` / `memory_correct_continue`, the surface
that forces a supersede regardless of embedding cosine). Treat every resolution
as "supersede the stale head with the corrected fact," not "delete."

---

## Commands and surfaces

```bash
quaid check --all --json               # contradiction detection across all pages
quaid check <slug> --json              # contradiction detection for one page
quaid validate --all --json            # referential integrity, stale embeddings, broken links
quaid gaps --json                      # unresolved knowledge gaps
quaid gaps --resolved --json           # already-resolved gaps (bare boolean flag)
quaid list --json                      # all pages (for orphan + stale review)
quaid graph <slug> --depth 1 --json    # inbound/outbound edges for one page
quaid compact                          # checkpoint the WAL into the main DB file
```

MCP equivalents an agent calls directly:

- `memory_check` — same heuristic detection as `quaid check`.
- `memory_correct` — open a correction dialogue against a fact slug; the SLM
  either commits a corrected fact (forcing a supersede of the prior head), asks
  one clarifying question, or abandons.
- `memory_correct_continue` — answer the clarifying question or abandon the open
  dialogue. A committed correction supersedes the stale head and stamps
  `supersedes` / `corrected_via` frontmatter on the successor.
- `memory_search` / `memory_query` — confirm the resolved head is what search
  now returns (pass `include_superseded: true` to see the retired head).

---

## Contradiction resolution workflow

```
1. Detect with `quaid check --all --json` (or the `memory_check` MCP tool).
   Each contradiction names a slug and a predicate with conflicting values.

2. Triage by severity:
   - Same predicate, mutually exclusive values (role = engineer vs manager)
     → resolve now.
   - Coexisting facts that only look contradictory (two valid phone numbers)
     → leave; they are not a contradiction, they are history.

3. Resolve via correction (NOT a manual edit):
   - Call memory_correct with:
       fact_slug   = the contradicting fact's slug
       correction  = a plain-language statement of the true current value
   - If the step outcome is "clarify", answer with memory_correct_continue
     (response = your clarification). Up to a few turns are allowed.
   - If the step outcome is "commit", the corrected fact is written and the
     prior head is superseded automatically — you do not edit pages by hand.
   - If the step outcome is "abandon", the dialogue could not produce an
     actionable fact; log a gap (see below) and move on.

4. Verify:
   - Re-run quaid check <slug> --json — the contradiction should be gone.
   - memory_search the predicate; confirm the corrected value is the head and
     the old value only appears with include_superseded: true.
```

Why supersede instead of edit: superseding preserves the timeline (the old fact
stays queryable as history) and records `corrected_via`, so the brain can
explain *why* it changed its mind. A manual overwrite loses that audit trail.

---

## Integrity validation

```
1. Run: quaid validate --all --json
2. Inspect the report's "passed" field and per-check violations:
   - links       → broken or dangling cross-references
   - assertions  → assertion rows with no backing page
   - embeddings  → rows referencing a non-active model_id (re-embed candidates)
3. For broken links, decide per edge: re-point to the correct slug
   (quaid link / memory_link) or close it (quaid unlink / memory_link_close).
4. For embedding drift, re-ingest or re-embed the affected pages so their
   vectors match the active model. Never silently delete embeddings.
```

---

## Knowledge gap triage

Gaps are queries the brain could not answer. They are the maintenance backlog.

```
1. List: quaid gaps --json
2. Prioritise by recurrence and recency — a gap logged many times is a real
   hole, a one-off may be noise.
3. For each high-value gap, either:
   - fill it: ingest or quaid put the missing knowledge, then the gap can be
     resolved; or
   - escalate it to the research skill if it needs external lookup (mind the
     sensitivity contract — do not send private query text outbound).
4. Confirm resolution: quaid gaps --resolved --json should now list it.
```

`--resolved` is a bare boolean flag (present = show resolved gaps); it takes no value.

---

## Orphan and stale-page review

```
1. Run: quaid list --json
2. Orphans: for each candidate, quaid graph <slug> --depth 1 --json and check
   for zero inbound AND zero outbound edges. An orphan is not automatically
   wrong — a standalone note can be legitimate — so surface it, do not delete it.
3. Stale heads: pages whose timeline has moved far past their compiled truth
   are candidates for re-compilation or a correction pass (see above).
```

---

## Compaction

After a maintenance pass that wrote pages (corrections, re-ingests), checkpoint
the write-ahead log back into the main database file:

```bash
quaid compact
```

This is safe to run any time; it does not change content.

---

## Exit conditions

- `quaid check --all` reports no unresolved contradictions.
- `quaid validate --all` passes (or every remaining violation is logged as a gap
  with a reason).
- High-value gaps are filled or escalated; the rest are recorded.
- The database has been compacted if pages were written.

## What this skill must NOT do

- Resolve a contradiction by editing or deleting a page directly — always go
  through the correction / supersede workflow so history and `corrected_via`
  provenance survive.
- Delete orphans or "stale" pages unattended — surface them for a human or
  agent decision.
- Send private gap query text to an external service — that is the research
  skill's job, under the sensitivity contract.
