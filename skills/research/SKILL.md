---
name: quaid-research
description: |
  Resolve knowledge gaps: fetch unresolved gaps, assess sensitivity, generate research
  queries, ingest findings, and close gaps with a resolution slug.
min_binary_version: "0.3.0"
---

# Research Skill

## Overview

The research skill provides a structured workflow for closing knowledge gaps logged in
the brain. Gaps are created with `sensitivity = 'internal'` by default. External research
requires an explicit approval step before any queries leave the local brain.

`memory_gap_approve` is an **approval workflow dependency**, not a binary command.
It represents a human or policy decision to escalate a gap's sensitivity from `internal`
to `external` or `redacted`. That approval may be granted by the user confirming in
their MCP client, a team policy workflow, or a future MCP tool when implemented.
Do not attempt to call `memory_gap_approve` as a CLI command — route escalation through
your session's approval mechanism.

---

## Sensitivity Levels

| Level | Meaning | Research Allowed |
|-------|---------|-----------------|
| `internal` | Brain-internal search only | `quaid query` + `quaid search` |
| `external` | Approved for web/API search | Exa, Crustdata, public APIs |
| `redacted` | External-approved but entity names stripped | Exa with anonymised query |

All gaps start as `internal`. Escalation to `external` or `redacted` requires approval.

---

## Workflow

### Step 1 — Fetch unresolved gaps

```bash
quaid gaps --limit 10 --json
```

Returns the ten highest-priority unresolved gaps. Each gap object contains:
- `id` — numeric gap identifier
- `query_text` — the original question (may be null if sensitivity is `redacted`)
- `query_hash` — SHA-256 of query (always present)
- `context` — additional context provided when the gap was logged
- `sensitivity` — `internal` / `external` / `redacted`
- `created_at`

### Step 2 — Assess each gap

For each gap:

1. **Is it already answerable?** Run `quaid query "<query_text>" --json` or
   `quaid search "<keywords>" --json`. If top result answers the question with
   confidence, proceed to Step 5 (resolve) without external research.

2. **What is the sensitivity?**
   - `internal` → proceed to Step 3a.
   - `external` → proceed to Step 3b.
   - `redacted` → proceed to Step 3c.

3. **Does it need priority escalation?** If the gap has high strategic importance
   (referenced by many pages, mentioned in a page with many inbound links), consider
   requesting approval to escalate to `external`.

### Step 3a — Internal-only research

Use brain-internal commands only:

```bash
quaid query "<query_text>" --limit 10 --json
quaid search "<keyword>" --json
quaid graph <related_slug> --depth 2 --json
```

Synthesise findings from results. If findings are sufficient, go to Step 4.
If not, and the gap warrants external research, request approval to escalate.

### Step 3b — External research (approved)

Only execute after the gap's `sensitivity` has been confirmed as `external` through
the approval workflow.

**Exa integration pattern:**

```
query = gap.query_text
POST https://api.exa.ai/search
Body: {"query": "<query>", "numResults": 5, "useAutoprompt": true}
Headers: {"x-api-key": "<EXA_API_KEY>"}
```

For each result:
- Retrieve full content: `POST https://api.exa.ai/contents` with result IDs
- Summarise content relevant to the gap question
- Store raw response via `memory_raw`:
  ```bash
  quaid call memory_raw '{"slug":"<target_slug>","source":"exa","data":<raw_json>}'
  ```

### Step 3c — Redacted external research

Sensitivity is `external` but entity names must be stripped before sending the query
externally. Construct a redacted query:

1. Take `gap.query_text` (or derive from `gap.context` if query_text is null).
2. Replace all proper nouns, company names, and person names with generic placeholders:
   - `<COMPANY>`, `<PERSON>`, `<PRODUCT>`.
3. Send the redacted query to Exa as in Step 3b.
4. Store results with `source = "exa_redacted"`.

### Step 4 — Ingest findings

Use the ingest skill to add findings to the brain. Two patterns:

**Pattern A — Write a new page:**
```bash
quaid put research/<gap_id>-findings < findings.md
```

**Pattern B — Update an existing page:**
```bash
quaid get <target_slug> --json   # fetch current version
# Edit compiled_truth with findings
# Append to timeline with source and date
quaid put <target_slug> --expected-version <N> < updated_page.md
```

Always append to the `timeline` section with a sourced, dated entry. Never overwrite
`compiled_truth` directly with raw external content — extract structured facts first.

### Step 5 — Mark gap resolved

Once findings are ingested and a resolution page/slug exists:

1. Obtain approval through the session's `memory_gap_approve` workflow, providing:
   - `gap_id`
   - `resolution_slug` — the slug of the page containing the answer

2. After approval is granted, confirm the gap appears in resolved state:
   ```bash
   quaid gaps --resolved true --json | jq '.[] | select(.id == <gap_id>)'
   ```

---

## Gap Prioritisation Heuristics

When deciding which gaps to research first:

1. Gaps referenced in pages with high inbound link counts (check `quaid graph`).
2. Gaps with `context` that mentions blocking work or decisions.
3. Gaps that are oldest (largest time since `created_at`).
4. Gaps with `external` sensitivity (already approved — no additional friction).

---

## Research Query Generation

When `query_text` is vague or too short to search effectively, expand it:

1. Take the gap's `context` field.
2. Run `quaid query "<context>" --json` to identify related pages.
3. Use the titles and summaries of related pages to construct specific sub-queries.
4. Break compound questions into individual factual questions (one query per unknown).

---

## Rate Limiting Guidance

| Source | Rate Limit | Guidance |
|--------|-----------|----------|
| Exa | 20 req/min (free tier) | Batch at most 10 gaps per session |
| Crustdata | Varies by tier | Store full API response in `memory_raw` first; extract facts second |
| GitHub API | 60 req/hr (unauthenticated) | Use authenticated token for upgrade skill integration |

Always store the raw API response in `memory_raw` before extracting facts. This ensures
idempotent re-extraction if the extraction logic improves later.

---

## Failure Modes

| Condition | Behaviour |
|-----------|-----------|
| `quaid gaps` returns empty | Log: "No unresolved gaps. Nothing to research." |
| Exa API returns 429 | Wait 60s and retry once; if still 429, skip gap and log warning |
| `memory_raw` returns `-32001` (slug not found) | Create target page first, then retry `memory_raw` |
| `memory_put` returns `ConflictError` | Re-fetch page (`quaid get <slug> --json`), merge changes, retry with new version |
| Approval for gap escalation is denied | Keep gap as `internal`; continue with internal-only research |
