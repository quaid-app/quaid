---
name: gbrain-alerts
description: |
  Interrupt-driven alerts: detect and surface new contradictions, stale pages,
  resolved gaps, and embedding drift. Priority-classified with deduplication.
min_binary_version: "0.3.0"
---

# Alerts Skill

## Overview

The alerts skill monitors brain state and surfaces actionable notifications.
It is designed to be called periodically (e.g., on a cron schedule or after
`brain_put` / `brain_link` operations) rather than interactively.

Alerts are written to **stdout as structured JSON** — one alert object per line
— so they can be piped to any downstream handler (logger, MCP client, dashboard).

---

## Alert Types and Priority Levels

| Alert Type | Priority | Trigger Condition |
|------------|----------|-------------------|
| `contradiction_new` | **high** | `gbrain check --all` returns a contradiction not seen in the previous run |
| `gap_resolved` | **low** | A gap transitions from `resolved_at IS NULL` to `resolved_at IS NOT NULL` |
| `page_stale` | **medium** | Page has `timeline_updated_at > truth_updated_at` by 30+ days AND has > 5 inbound links |
| `embedding_drift` | **low** | Any `page_embeddings` row references a `model_id` that is not the current active model |

Priority ladder (highest to lowest): `critical` → `high` → `medium` → `low`.
Currently no trigger reaches `critical`; reserve that level for future use
(e.g., data corruption detected by `gbrain validate`).

---

## Commands

The alerts skill orchestrates existing commands. There is no standalone `gbrain alerts`
command; the agent runs these and synthesises results.

```bash
# Check for new contradictions
gbrain check --all --json

# Check for recently resolved gaps (compare against last known state)
gbrain gaps --resolved true --json

# List pages for stale-risk check
gbrain list --json
gbrain graph <slug> --depth 1 --json   # check inbound link count per candidate page

# Check embedding model consistency
gbrain validate --embeddings --json
```

---

## Alert Object Schema

Each alert is a JSON object with this structure:

```json
{
  "type": "contradiction_new",
  "priority": "high",
  "slug": "people/alice",
  "message": "Contradiction on predicate 'role' between 'engineer' and 'manager'",
  "detected_at": "2026-04-17T09:00:00Z",
  "dedup_key": "contradiction_new::people/alice::role"
}
```

Fields:
- `type` — one of the alert types in the table above
- `priority` — `critical` / `high` / `medium` / `low`
- `slug` — the page or resource affected (empty string if not page-scoped)
- `message` — human-readable description
- `detected_at` — ISO-8601 UTC timestamp
- `dedup_key` — opaque string used for suppression (see Deduplication)

---

## Deduplication Rules

The agent MUST maintain a deduplication log (a simple key-value store, a brain page,
or a local file). The default suppression window is **24 hours**.

**Rule:** If an alert with the same `dedup_key` was emitted within the suppression window,
do NOT emit it again. Discard silently.

**Dedup key construction:**

| Alert Type | Dedup Key Pattern |
|------------|-------------------|
| `contradiction_new` | `contradiction_new::<slug>::<predicate>` |
| `gap_resolved` | `gap_resolved::<gap_id>` |
| `page_stale` | `page_stale::<slug>` |
| `embedding_drift` | `embedding_drift::global` |

If a contradiction is resolved and then re-detected, the dedup key changes because the
predicate value pair changes — it will fire again. This is intentional.

---

## Detection Workflows

### Contradiction alerts

```
1. Run: gbrain check --all --json
2. For each contradiction in the response:
   a. Compute dedup_key = "contradiction_new::<slug>::<predicate>"
   b. Check suppression log for key within last 24h
   c. If NOT suppressed: emit alert, record key + timestamp in log
```

### Stale page alerts

```
1. Run: gbrain list --json
2. Filter pages where:
     (now - timeline_updated_at > 30 days) AND (truth_updated_at < timeline_updated_at)
3. For each candidate slug:
   a. Run: gbrain graph <slug> --depth 1 --json
   b. Count inbound links (edges where target == slug)
   c. If inbound_links > 5:
      - Compute dedup_key = "page_stale::<slug>"
      - Check suppression log; if NOT suppressed: emit medium alert
```

### Gap resolved alerts

```
1. Run: gbrain gaps --resolved true --json
2. For each gap with resolved_at != null:
   a. Compute dedup_key = "gap_resolved::<gap_id>"
   b. If NOT in suppression log: emit low alert, record key
```

### Embedding drift alerts

```
1. Run: gbrain validate --embeddings --json
2. Parse "passed" field
3. If passed == false:
   a. Compute dedup_key = "embedding_drift::global"
   b. If NOT suppressed within 24h: emit low alert with message "Re-embed recommended"
```

---

## Suppression Configuration

Agents may configure the suppression window per alert type:

```yaml
suppression_windows:
  contradiction_new: 24h
  gap_resolved: 72h      # gaps resolve slowly; don't spam
  page_stale: 168h       # stale pages don't change fast; suppress for 7 days
  embedding_drift: 24h
```

These are agent-side configuration values, not binary flags.

---

## Output Delivery

Write one JSON alert object per line to stdout:

```
{"type":"contradiction_new","priority":"high","slug":"people/alice",...}
{"type":"page_stale","priority":"medium","slug":"companies/acme",...}
```

Downstream handlers consume this stream. Examples:
- Log to a brain page: `gbrain alerts | gbrain put logs/alerts-<date>`
- Filter by priority: `gbrain alerts | jq 'select(.priority == "high")'`
- Count today's alerts: `gbrain alerts | jq -s 'length'`

---

## Failure Modes

| Condition | Behaviour |
|-----------|-----------|
| `gbrain check --all` DB error | Emit one `critical`-priority alert with `type: "check_failed"` and the error message |
| `gbrain validate --embeddings` fails to run | Skip embedding drift check; log warning to stderr |
| Suppression log unreadable | Emit all alerts (fail open — missing suppression is safer than missing alerts) |
| No brain pages exist | All checks return empty; emit no alerts |
