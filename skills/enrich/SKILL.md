---
name: gbrain-enrich
description: |
  Enrich brain pages with external data from Crustdata, Exa, and Partiful.
  Stores raw API responses, extracts structured facts, and handles conflicts.
min_binary_version: "0.3.0"
---

# Enrich Skill

## Overview

The enrich skill integrates external data sources into brain pages. Raw API responses
are stored atomically in the `raw_data` table via `brain_raw`, then structured facts
are extracted and written into `compiled_truth` and `assertions`. This two-phase approach
makes enrichment idempotent and allows re-extraction if extraction logic improves.

**Rule:** Never overwrite `compiled_truth` directly with raw external content.
Always extract facts first and flag conflicts as contradictions.

---

## Data Sources

### Crustdata — Company and person professional data

Crustdata provides firmographic data: funding rounds, headcount, leadership roles,
revenue signals, and professional histories.

**Endpoint:** `https://api.crustdata.com/v1/companies/search` (and `/people/search`)

**Rate limit:** Varies by subscription tier. Cache all responses in `brain_raw` to avoid
repeated API calls.

### Exa — Web search and content extraction

Exa provides neural web search with full-page content extraction, suitable for research
queries and enriching pages with recent public information.

**Endpoints:**
- `POST https://api.exa.ai/search` — search
- `POST https://api.exa.ai/contents` — fetch full content by result IDs

**Rate limit:** 20 req/min on free tier. Use `brain_raw` to cache responses.

### Partiful — Event and social data

Partiful provides event attendance, RSVPs, and social network data for community-focused
knowledge graphs.

**Usage:** Partiful does not have a public REST API; extract structured data from event
export files or invitation CSVs. Import as pages via `gbrain import`, then enrich with
the flow below.

---

## Storage Flow

### Phase 1 — Store raw response

Store the full API response before doing any extraction. This is the idempotency anchor.

```bash
gbrain call brain_raw '{
  "slug": "<page_slug>",
  "source": "<crustdata|exa|partiful>",
  "data": <raw_api_response_json>
}'
```

Returns: `{"id": <row_id>}`

If the target page does not exist yet, create it first:
```bash
gbrain put <page_slug> < stub.md   # minimal page: slug + type only
```

### Phase 2 — Extract facts

Read the stored raw data and derive structured facts:

1. Identify factual claims in the API response (funding amount, headcount, role, etc.).
2. For each fact, decide:
   - Does it update an existing field in `compiled_truth`? → Check for conflict (see below).
   - Is it a new fact? → Append to `compiled_truth`.
   - Is it time-bounded (e.g., "held role X from 2022 to 2024")? → Append to `timeline`.

### Phase 3 — Write updated page

```bash
gbrain get <page_slug> --json     # fetch current page + version
# Merge extracted facts into compiled_truth and timeline
gbrain put <page_slug> --expected-version <N> < updated.md
```

Always use `--expected-version` to detect concurrent writes (OCC). If the write returns
`ConflictError`, re-fetch and merge again.

---

## Enrichment Patterns by Source

### Crustdata — Company page enrichment

**Target page type:** `companies/<slug>`

**Facts to extract and store in `compiled_truth`:**
- `headcount` — employee count (latest reported)
- `funding_total` — total funding raised (USD)
- `last_funding_round` — series + date
- `hq_location` — city, country
- `founded` — year

**Timeline entries to append:**
- Each funding round: `YYYY-MM-DD: Series <X> — $<amount>`
- Leadership changes if available: `YYYY-MM-DD: <Person> joined as <Role>`

**Example workflow:**
```
1. GET https://api.crustdata.com/v1/companies/search?domain=acme.com
2. gbrain call brain_raw '{"slug":"companies/acme","source":"crustdata","data":<response>}'
3. Extract: headcount=450, funding_total=$42M, last_round="Series B 2024-03"
4. gbrain get companies/acme --json → fetch + version
5. Merge facts into compiled_truth; append funding round to timeline
6. gbrain put companies/acme --expected-version <N> < updated.md
```

### Crustdata — Person page enrichment

**Target page type:** `people/<slug>`

**Facts to extract:**
- `current_role` — title at current employer
- `current_company` — employer slug
- `seniority` — IC / manager / exec
- `location` — city, country

**Relationships to create:**
```bash
gbrain link people/<slug> companies/<employer_slug> \
  --relationship works_at \
  --valid-from <start_year>
```

### Exa — Research enrichment

**Use case:** Fill knowledge gaps with recent public information.

```
1. query = gap.query_text or derived research question
2. POST https://api.exa.ai/search with query
3. For top 3 results: POST https://api.exa.ai/contents with result IDs
4. Store each result: gbrain call brain_raw '{"slug":"<target>","source":"exa","data":<result>}'
5. Extract key facts; append to compiled_truth with source citation
6. Append timeline entry: "YYYY-MM-DD: [Exa] <summary> (source: <url>)"
```

Always include the source URL as a citation in the timeline entry. Never assert something
as `compiled_truth` without a cited source from external enrichment.

### Partiful — Event page enrichment

**Target page type:** `events/<slug>` or `people/<slug>`

For event pages:
- Extract attendee list → create `people/<slug>` stubs for new attendees
- Create `attended` links from person pages to event page
- Store RSVP data in `raw_data`

```bash
gbrain call brain_raw '{"slug":"events/<event_slug>","source":"partiful","data":<export>}'
# For each attendee not already in brain:
gbrain put people/<attendee_slug> < stub.md
gbrain link people/<attendee_slug> events/<event_slug> --relationship attended --valid-from <event_date>
```

---

## Conflict Resolution

When enrichment data contradicts existing `compiled_truth`:

1. **Do NOT overwrite** the existing value automatically.
2. Use `brain_check` or inspect existing assertions to understand the conflict.
3. Log the contradiction explicitly:
   - Old value: existing `compiled_truth` statement
   - New value: enrichment data claim
   - Source: API source and timestamp
4. Add to the `timeline` section:
   ```
   YYYY-MM-DD: [Conflict flagged] Crustdata reports headcount=450; brain has 380 (as of 2025-01).
   ```
5. The agent (or user) resolves by:
   - Accepting the new value (update `compiled_truth`, move old value to timeline)
   - Rejecting the new value (add a note in timeline, keep `compiled_truth` unchanged)
   - Marking as ambiguous (add both values to compiled_truth with source notes)

The resolution decision is always agent/user-driven. This skill does not auto-accept
external data as ground truth.

---

## Rate Limiting Guidance

| Source | Guidance |
|--------|----------|
| Crustdata | Store full response in `brain_raw` before extraction. Batch enrichment to ≤ 20 pages per session. |
| Exa | 20 req/min free tier. Introduce 3s delay between requests. Cache all results. |
| Partiful | File-based; no API rate limit. Process event exports one at a time. |

---

## Failure Modes

| Condition | Behaviour |
|-----------|-----------|
| `brain_raw` returns `-32001` | Target page does not exist — create stub page first, then retry |
| `brain_put` returns `ConflictError` | Re-fetch page with `gbrain get --json`, merge changes, retry with new version |
| Crustdata / Exa returns 429 | Wait 60s, retry once. If still 429, skip page and log: `Skipped <slug>: rate limited` |
| API returns empty results | Log: `No enrichment data found for <slug> from <source>`. Skip gracefully. |
| Extracted fact is ambiguous | Log conflict in timeline; do not write to `compiled_truth` |
