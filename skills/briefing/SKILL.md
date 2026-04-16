---
name: gbrain-briefing
description: |
  Generate a structured "what shifted" report from the brain: changed pages,
  new pages, unresolved contradictions, open knowledge gaps, and upcoming timeline entries.
  Configurable lookback window; default is 1 day.
min_binary_version: "0.3.0"
---

# Briefing Skill

## Overview

The briefing skill generates a structured intelligence report covering everything that has
changed in the brain since a given cutoff. Run it daily (or on demand) to stay current
without reading every page individually.

Default lookback: **1 day**. Override with `--days N`.

---

## Commands

```bash
gbrain query "pages updated in last 24 hours" --json
gbrain check --all --json
gbrain gaps --limit 10 --json
gbrain list --json
```

There is no single `gbrain briefing` command — the skill orchestrates existing commands
and synthesises their output into the report structure below.

---

## Briefing Sections

### 1. What Shifted

Pages where `truth_updated_at` or `timeline_updated_at` falls within the lookback window.

**Agent invocation:**
```bash
gbrain list --json | jq '[.[] | select(.truth_updated_at > "<CUTOFF>" or .timeline_updated_at > "<CUTOFF>")]'
```

Replace `<CUTOFF>` with an ISO-8601 timestamp calculated as `now - lookback_days`.

For each shifted page, include:
- Slug
- Last updated timestamp (whichever is more recent)
- One-line summary from `compiled_truth` (first non-blank line)

Rank by recency (most recent first). Show at most 20 pages unless the agent is instructed
to show all.

### 2. New Pages

Pages created within the lookback window (`created_at` field).

```bash
gbrain list --json | jq '[.[] | select(.created_at > "<CUTOFF>")]'
```

Include slug, type (wing), and the first sentence of `compiled_truth`. Rank by `created_at`
descending.

### 3. Unresolved Contradictions

```bash
gbrain check --all --json
```

Output the full list of contradiction objects from the response. If zero contradictions,
print `✓ No contradictions detected.` For each contradiction include:
- Slug of the affected page
- Predicate in conflict
- Conflicting values (brief)
- Detected at timestamp

### 4. Knowledge Gaps

```bash
gbrain gaps --limit 10 --json
```

List the ten highest-priority unresolved gaps. For each gap include:
- Gap ID
- Query text (may be redacted if `sensitivity = 'redacted'`)
- Sensitivity level
- Created at timestamp

If sensitivity is `redacted`, show `[query redacted]` instead of the query text.

### 5. Upcoming

Pages whose `timeline` section contains future-dated entries (entries with a date string
greater than today's date). Retrieve with:

```bash
gbrain query "timeline entries upcoming future" --limit 20 --json
```

The agent should parse returned page timelines for entries matching the pattern
`YYYY-MM-DD` and filter to those > today. Show up to 10 upcoming items with:
- Slug
- Date
- Entry summary (first line of the timeline entry)

---

## Output Format

Produce the briefing as a markdown document with this structure:

```markdown
# Brain Briefing — <DATE>
*Lookback: last <N> day(s)*

## What Shifted
...

## New Pages
...

## Contradictions
...

## Knowledge Gaps
...

## Upcoming
...
```

---

## Configurable Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `--days N` | `1` | Lookback window in days |
| `--wing <wing>` | all wings | Restrict briefing to one palace wing |
| `--limit N` | `20` | Max shifted pages to show |
| `--gaps-limit N` | `10` | Max knowledge gaps to list |
| `--json` | false | Output briefing sections as structured JSON instead of markdown |

---

## Example Agent Invocation Sequence

1. Compute the cutoff timestamp: `now - days * 86400` as ISO-8601.
2. Run `gbrain list --json` → filter by `truth_updated_at` or `timeline_updated_at` > cutoff → **What Shifted** and **New Pages**.
3. Run `gbrain check --all --json` → **Contradictions**.
4. Run `gbrain gaps --limit 10 --json` → **Knowledge Gaps**.
5. Run `gbrain query "upcoming timeline entries" --json` → parse for future dates → **Upcoming**.
6. Assemble and render the briefing markdown document.
7. Optionally persist the briefing as a new brain page: `gbrain put briefings/<YYYY-MM-DD>`.

---

## Prioritisation Heuristics

When too many pages shifted to show all:

1. Prefer pages with **both** `truth_updated_at` and `timeline_updated_at` updated (double signal).
2. Prefer pages with **inbound links** (high connectivity = high importance).
3. Prefer pages in `people/` and `companies/` wings over `raw/` and `ingest/`.
4. If still over limit, truncate and append `...(N more pages shifted)`.

---

## Failure Modes

| Condition | Behaviour |
|-----------|-----------|
| Brain empty | Each section reports "No data." |
| `gbrain check --all` returns DB error | Log error in Contradictions section; continue |
| `gbrain gaps` returns empty | Print `✓ No open gaps.` |
| ISO timestamp parse error | Abort and report: `Error: invalid cutoff timestamp` |
